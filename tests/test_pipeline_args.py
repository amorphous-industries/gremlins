from __future__ import annotations

import json
import logging

import pytest

from gremlins.cli.pipeline_args import (
    _load_global_config,
    launch_client_label,
    load_prefix_clients,
)


class TestLoadGlobalConfig:
    def test_returns_empty_dict_when_no_file(self, sandbox):
        assert _load_global_config() == {}

    def test_returns_parsed_dict(self, sandbox):
        sandbox.config.mkdir(parents=True, exist_ok=True)
        (sandbox.config / "config.json").write_text(
            json.dumps({"default-client": "openai:gpt-4o"})
        )
        assert _load_global_config() == {"default-client": "openai:gpt-4o"}

    def test_raises_on_non_dict_top_level(self, sandbox):
        sandbox.config.mkdir(parents=True, exist_ok=True)
        (sandbox.config / "config.json").write_text("[1, 2, 3]")
        with pytest.raises(ValueError, match="JSON object"):
            _load_global_config()

    def test_raises_on_malformed_json(self, sandbox):
        sandbox.config.mkdir(parents=True, exist_ok=True)
        (sandbox.config / "config.json").write_text("{bad")
        with pytest.raises(ValueError, match="config.json is not valid JSON"):
            _load_global_config()


class TestLoadPrefixClients:
    def test_returns_empty_when_no_file(self, sandbox):
        assert load_prefix_clients() == {}

    def test_returns_empty_when_no_prefix_keys(self, sandbox):
        sandbox.config.mkdir(parents=True, exist_ok=True)
        (sandbox.config / "config.json").write_text(
            json.dumps({"default-client": "a:b"})
        )
        assert load_prefix_clients() == {}

    def test_extracts_prefix_rules(self, sandbox):
        sandbox.config.mkdir(parents=True, exist_ok=True)
        (sandbox.config / "config.json").write_text(
            json.dumps(
                {
                    "default-client": "a:b",
                    "default-client-by-stage": {
                        "local-review-*": "openrouter:doomclientv5",
                        "plan-*": "openai:gpt-5",
                    },
                }
            )
        )
        result = load_prefix_clients()
        assert result == {
            "local-review-": "openrouter:doomclientv5",
            "plan-": "openai:gpt-5",
        }

    def test_ignores_non_string_values(self, sandbox, caplog):
        sandbox.config.mkdir(parents=True, exist_ok=True)
        (sandbox.config / "config.json").write_text(
            json.dumps(
                {
                    "default-client-by-stage": {
                        "prefix-*": 42,
                        "valid-*": "openrouter:model",
                    },
                }
            )
        )
        with caplog.at_level(logging.WARNING):
            result = load_prefix_clients()
        assert result == {"valid-": "openrouter:model"}
        assert "non-string value" in caplog.text
        assert "prefix-*" in caplog.text

    def test_skips_key_without_trailing_star(self, sandbox, caplog):
        sandbox.config.mkdir(parents=True, exist_ok=True)
        (sandbox.config / "config.json").write_text(
            json.dumps(
                {
                    "default-client-by-stage": {
                        "local-review": "openrouter:model",
                        "plan-*": "openai:gpt-5",
                    },
                }
            )
        )
        with caplog.at_level(logging.WARNING):
            result = load_prefix_clients()
        assert result == {"plan-": "openai:gpt-5"}
        assert "missing a trailing '*'" in caplog.text
        assert "local-review" in caplog.text

    def test_skips_empty_prefix_from_bare_star(self, sandbox, caplog):
        sandbox.config.mkdir(parents=True, exist_ok=True)
        (sandbox.config / "config.json").write_text(
            json.dumps(
                {
                    "default-client-by-stage": {
                        "*": "openrouter:model",
                        "plan-*": "openai:gpt-5",
                    },
                }
            )
        )
        with caplog.at_level(logging.WARNING):
            result = load_prefix_clients()
        assert result == {"plan-": "openai:gpt-5"}
        assert "empty prefix" in caplog.text

    def test_strips_trailing_star_only(self, sandbox):
        sandbox.config.mkdir(parents=True, exist_ok=True)
        (sandbox.config / "config.json").write_text(
            json.dumps({"default-client-by-stage": {"local-*": "openrouter:model"}})
        )
        result = load_prefix_clients()
        assert "local-" in result
        assert result["local-"] == "openrouter:model"


class TestLaunchClientLabel:
    def test_cli_flag_wins(self, sandbox):
        result = launch_client_label(["--client", "a:b"], FakePipeline("e:f"))
        assert result == "a:b"

    def test_cli_flag_equals_form_wins(self, sandbox):
        result = launch_client_label(["--client=c:d"], FakePipeline("e:f"))
        assert result == "c:d"

    def test_global_config_beats_pipeline(self, sandbox):
        sandbox.config.mkdir(parents=True, exist_ok=True)
        (sandbox.config / "config.json").write_text(
            json.dumps({"default-client": "c:d"})
        )
        result = launch_client_label([], FakePipeline("e:f"))
        assert result == "c:d"

    def test_pipeline_default_is_fallback(self, sandbox):
        result = launch_client_label([], FakePipeline("e:f"))
        assert result == "e:f"

    def test_error_when_none_set(self, sandbox):
        with pytest.raises(ValueError, match="no client configured"):
            launch_client_label([], None)

    def test_global_config_empty_default_client_falls_through(self, sandbox):
        sandbox.config.mkdir(parents=True, exist_ok=True)
        (sandbox.config / "config.json").write_text(json.dumps({"default-client": ""}))
        result = launch_client_label([], FakePipeline("e:f"))
        assert result == "e:f"


class FakePipeline:
    def __init__(self, default_client):
        self.default_client = default_client
