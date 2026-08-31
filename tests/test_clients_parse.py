"""Tests for Client.parse contract."""

import pytest
from _gremlins_core.clients import RustClient as Client


def test_parse_xai():
    c = Client.parse("xai:grok-4")
    assert c.provider == "xai"
    assert c.model == "grok-4"
    assert str(c) == "xai:grok-4"


def test_parse_openai():
    c = Client.parse("openai:gpt-4o-mini")
    assert c.provider == "openai"
    assert c.model == "gpt-4o-mini"
    assert str(c) == "openai:gpt-4o-mini"


def test_parse_cmd():
    c = Client.parse(
        "cmd:claude -p --model sonnet --verbose --output-format stream-json --permission-mode bypassPermissions"
    )
    assert c.provider == "cmd"
    assert (
        c.model
        == "claude -p --model sonnet --verbose --output-format stream-json --permission-mode bypassPermissions"
    )


def test_parse_roundtrips():
    for spec in (
        "cmd:claude -p --model sonnet --verbose --output-format stream-json --permission-mode bypassPermissions",
        "openai:gpt-4o-mini",
        "xai:grok-4",
        "openrouter:openai/gpt-4o",
        "openrouter:deepseek/deepseek-v4-pro:reasoning=high",
        "openrouter:deepseek/deepseek-v4-pro:reasoning=high,thinking=deepseek",
        "xai:grok-4:reasoning=low,foo=bar",
    ):
        assert str(Client.parse(spec)) == spec


def test_parse_with_reasoning_params():
    c = Client.parse("openrouter:deepseek/deepseek-v4-pro:reasoning=high")
    assert c.provider == "openrouter"
    assert c.model == "deepseek/deepseek-v4-pro"
    assert c.extra_params == {"reasoning": "high"}


def test_parse_with_multiple_params():
    c = Client.parse(
        "openrouter:deepseek/deepseek-v4-pro:reasoning=high,thinking=deepseek,foo=bar"
    )
    assert c.provider == "openrouter"
    assert c.model == "deepseek/deepseek-v4-pro"
    assert c.extra_params == {
        "reasoning": "high",
        "thinking": "deepseek",
        "foo": "bar",
    }


def test_parse_cmd_ignores_params_suffix():
    # cmd provider treats the entire rest as the model — no param parsing.
    c = Client.parse("cmd:echo hello:world=foo")
    assert c.provider == "cmd"
    assert c.model == "echo hello:world=foo"
    assert c.extra_params == {}


def test_parse_no_params():
    c = Client.parse("openai:gpt-4o-mini")
    assert c.provider == "openai"
    assert c.model == "gpt-4o-mini"
    assert c.extra_params == {}


def test_parse_no_colon_raises():
    with pytest.raises(ValueError, match="'provider:model'"):
        Client.parse("no-colon")


def test_parse_empty_provider_raises():
    with pytest.raises(ValueError, match="provider must not be empty"):
        Client.parse(":model")


def test_parse_empty_model_raises():
    with pytest.raises(ValueError, match="model"):
        Client.parse("provider:")


def test_parse_unknown_provider_raises():
    with pytest.raises(ValueError, match="unknown provider"):
        Client.parse("does-not-exist:foo")


def test_parse_duplicate_key_raises():
    with pytest.raises(ValueError, match="duplicate key"):
        Client.parse("openai:gpt-4:reasoning=high,reasoning=low")


def test_client_equality_and_hash():
    a = Client("openai", "gpt-4")
    b = Client("openai", "gpt-4")
    c = Client("openai", "gpt-4o")
    d = Client("openai", "gpt-4", extra_params={"reasoning": "high"})
    e = Client("openai", "gpt-4", extra_params={"reasoning": "high"})
    f = Client("openai", "gpt-4", extra_params={"reasoning": "low"})
    assert a == b
    assert hash(a) == hash(b)
    assert a != c
    assert hash(a) != hash(c)
    # extra_params affect equality
    assert a != d
    assert d == e
    assert hash(d) == hash(e)
    assert d != f
