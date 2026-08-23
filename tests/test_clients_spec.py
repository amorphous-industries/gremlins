"""Tests for Client parsing and stage client resolution."""

from __future__ import annotations

import pytest

from gremlins.clients.client import PACKAGE_DEFAULT, Client


def test_parse_valid():
    spec = Client.parse("openai:gpt-4o")
    assert spec.provider == "openai"
    assert spec.model == "gpt-4o"


def test_parse_empty_model():
    with pytest.raises(ValueError, match="model must not be empty"):
        Client.parse("openai:")


def test_parse_no_colon_raises():
    with pytest.raises(ValueError, match="expected 'provider:model'"):
        Client.parse("openai")


def test_parse_unknown_provider_raises():
    with pytest.raises(ValueError, match="unknown provider"):
        Client.parse("unknown:model")


def test_str_round_trip():
    for s in ("openai:gpt-4o", "xai:grok-4"):
        assert str(Client.parse(s)) == s


def test_package_default():
    assert PACKAGE_DEFAULT.provider == "cmd"
    assert (
        PACKAGE_DEFAULT.model
        == "claude -p --model sonnet --verbose --output-format stream-json --permission-mode bypassPermissions"
    )
