from __future__ import annotations

import json

import pytest

from gremlins.cli.pipeline_args import _load_global_config, launch_client_label


class TestLoadGlobalConfig:
    def test_returns_empty_dict_when_no_file(self, sandbox):
        assert _load_global_config() == {}

    def test_returns_parsed_dict(self, sandbox):
        sandbox.config.mkdir(parents=True, exist_ok=True)
        (sandbox.config / "config.json").write_text(
            json.dumps({"default_client": "openai:gpt-4o"})
        )
        assert _load_global_config() == {"default_client": "openai:gpt-4o"}

    def test_raises_on_non_dict_top_level(self, sandbox):
        sandbox.config.mkdir(parents=True, exist_ok=True)
        (sandbox.config / "config.json").write_text("[1, 2, 3]")
        with pytest.raises(ValueError, match="JSON object"):
            _load_global_config()

    def test_raises_on_malformed_json(self, sandbox):
        sandbox.config.mkdir(parents=True, exist_ok=True)
        (sandbox.config / "config.json").write_text("{bad")
        with pytest.raises(ValueError, match="Expecting"):
            _load_global_config()


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
            json.dumps({"default_client": "c:d"})
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
        (sandbox.config / "config.json").write_text(json.dumps({"default_client": ""}))
        result = launch_client_label([], FakePipeline("e:f"))
        assert result == "e:f"


class FakePipeline:
    def __init__(self, default_client):
        self.default_client = default_client
