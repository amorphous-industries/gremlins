"""Tests for resolve_interpolation_map ?default syntax (E2)."""

from __future__ import annotations

from unittest.mock import MagicMock

import pytest

from gremlins.artifacts.registry import MissingArtifact
from gremlins.artifacts.resolve import resolve_interpolation_map


def _registry(bindings: dict) -> MagicMock:
    reg = MagicMock()

    def _read(key):
        if key not in bindings:
            raise MissingArtifact(key)
        return bindings[key]

    reg.read.side_effect = _read
    return reg


def test_bound_key_default_ignored():
    reg = _registry({"k": "live-value"})
    assert resolve_interpolation_map(reg, {"v": "k?fallback"}) == {"v": "live-value"}


def test_unbound_key_empty_default():
    reg = _registry({})
    assert resolve_interpolation_map(reg, {"v": "missing?"}) == {"v": ""}


def test_unbound_key_literal_default():
    reg = _registry({})
    assert resolve_interpolation_map(reg, {"v": "missing?main"}) == {"v": "main"}


def test_unbound_key_fallback_with_literal_dot_in_name():
    """Dot-prefixed path syntax is no longer special — dots are literal key chars."""
    reg = _registry({"pr": {"branch": "feat"}})
    assert resolve_interpolation_map(reg, {"v": "pr.brnch?fallback"}) == {
        "v": "fallback"
    }


def test_no_default_missing_artifact_raises():
    reg = _registry({})
    with pytest.raises(MissingArtifact):
        resolve_interpolation_map(reg, {"v": "missing"})


def test_missing_artifact_raises_on_literal_dot_key():
    """Dots in keys are literal — 'ref.name' is a single key, not a traversal."""
    reg = _registry({"ref": {"name": "main"}})
    with pytest.raises(MissingArtifact):
        resolve_interpolation_map(reg, {"v": "ref.name"})
