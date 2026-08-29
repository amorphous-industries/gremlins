"""Tests for baked-in prefix-based client assignment at launch time."""

from __future__ import annotations

import json

from gremlins.cli.pipeline_args import load_prefix_clients
from gremlins.launcher import _bake_prefix_clients


def _expanded_stages(stages: list[dict]) -> dict:
    """Return a minimal expanded pipeline dict wrapping the given stages."""
    return {"stages": stages}


def test_no_prefix_map_leaves_stages_unchanged():
    stages = [{"name": "review-one"}, {"name": "plan"}]
    _bake_prefix_clients(_expanded_stages(stages), {})
    assert "client" not in stages[0]
    assert "client" not in stages[1]


def test_matching_prefix_adds_client():
    stages = [{"name": "local-review-one"}, {"name": "local-review-two"}]
    _bake_prefix_clients(
        _expanded_stages(stages), {"local-review-": "openrouter:doomclientv5"}
    )
    assert stages[0].get("client") == "openrouter:doomclientv5"
    assert stages[1].get("client") == "openrouter:doomclientv5"


def test_non_matching_stage_unchanged():
    stages = [{"name": "plan"}, {"name": "implement"}]
    _bake_prefix_clients(
        _expanded_stages(stages), {"local-review-": "openrouter:doomclientv5"}
    )
    assert "client" not in stages[0]
    assert "client" not in stages[1]


def test_respects_existing_client_in_yaml():
    stages = [
        {"name": "local-review-one", "client": "openai:gpt-5"},
        {"name": "local-review-two"},
    ]
    _bake_prefix_clients(
        _expanded_stages(stages), {"local-review-": "openrouter:doomclientv5"}
    )
    assert stages[0].get("client") == "openai:gpt-5", "explicit YAML client preserved"
    assert stages[1].get("client") == "openrouter:doomclientv5"


def test_recursive_into_parallel():
    stages = [
        {
            "name": "group",
            "parallel": [
                {"name": "local-review-a"},
                {"name": "other"},
            ],
        }
    ]
    _bake_prefix_clients(
        _expanded_stages(stages), {"local-review-": "openrouter:doomclientv5"}
    )
    children = stages[0]["parallel"]
    assert children[0].get("client") == "openrouter:doomclientv5"
    assert "client" not in children[1]


def test_recursive_into_body():
    stages = [
        {
            "name": "loop",
            "body": [
                {"name": "local-review-a"},
                {"name": "other"},
            ],
        }
    ]
    _bake_prefix_clients(
        _expanded_stages(stages), {"local-review-": "openrouter:doomclientv5"}
    )
    children = stages[0]["body"]
    assert children[0].get("client") == "openrouter:doomclientv5"
    assert "client" not in children[1]


def test_respects_existing_client_in_parallel_child():
    stages = [
        {
            "name": "group",
            "parallel": [
                {"name": "local-review-a", "client": "openai:gpt-5"},
                {"name": "local-review-b"},
            ],
        }
    ]
    _bake_prefix_clients(
        _expanded_stages(stages), {"local-review-": "openrouter:doomclientv5"}
    )
    children = stages[0]["parallel"]
    assert children[0].get("client") == "openai:gpt-5", "explicit client preserved in child"
    assert children[1].get("client") == "openrouter:doomclientv5"


def test_multiple_prefixes_first_match_wins():
    stages = [
        {"name": "review-code"},
        {"name": "local-review-one"},
        {"name": "plan"},
    ]
    _bake_prefix_clients(
        _expanded_stages(stages),
        {
            "local-review-": "openrouter:doomclientv5",
            "review-": "openai:gpt-5",
        },
    )
    assert stages[0].get("client") == "openai:gpt-5"
    assert stages[1].get("client") == "openrouter:doomclientv5"
    assert "client" not in stages[2]


def test_noop_on_empty_stages():
    _bake_prefix_clients(_expanded_stages([]), {"local-review-": "openrouter:doomclientv5"})


class TestIntegrationWithGlobalConfig:
    def test_load_and_bake_prefixes(self, sandbox):
        sandbox.config.mkdir(parents=True, exist_ok=True)
        (sandbox.config / "config.json").write_text(
            json.dumps(
                {
                    "default-client": "openai:gpt-4o",
                    "default-client-by-stage": {
                        "local-review-*": "openrouter:doomclientv5",
                        "plan-*": "openai:gpt-5",
                    },
                }
            )
        )
        prefixes = load_prefix_clients()
        assert prefixes == {
            "local-review-": "openrouter:doomclientv5",
            "plan-": "openai:gpt-5",
        }

        stages = [
            {"name": "plan-one"},
            {"name": "local-review-one"},
            {"name": "implement"},
        ]
        _bake_prefix_clients(_expanded_stages(stages), prefixes)
        assert stages[0].get("client") == "openai:gpt-5"
        assert stages[1].get("client") == "openrouter:doomclientv5"
        assert "client" not in stages[2]