"""Tests for Client parsing and stage client resolution."""

from __future__ import annotations

import pytest

from gremlins.clients import _DEFAULT_ALLOWED_TOOLS
from gremlins.clients.client import Client


def test_default_allowlist_has_expected_tools():
    """The six-tool allowlist is the source of truth for all SDK backends."""
    assert _DEFAULT_ALLOWED_TOOLS == [
        "Bash",
        "Edit",
        "Read",
        "Write",
        "Grep",
        "Glob",
    ]


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
