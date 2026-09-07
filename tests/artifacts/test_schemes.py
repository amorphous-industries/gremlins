"""Tests for gremlins.artifacts.schemes."""

from __future__ import annotations

import pathlib

import pytest
from _gremlins_core.artifacts import Uri

from gremlins.artifacts.schemes import FileArtifactResolver


# FileArtifactResolver tests


def test_file_resolver_read(tmp_path: pathlib.Path) -> None:
    (tmp_path / "out.txt").write_text("content", encoding="utf-8")
    resolver = FileArtifactResolver(tmp_path)
    uri = Uri(scheme="file", path="session/out.txt")
    assert resolver.read(uri) == "content"


def test_file_resolver_verify_produced_raises_when_absent(
    tmp_path: pathlib.Path,
) -> None:
    resolver = FileArtifactResolver(tmp_path)
    uri = Uri(scheme="file", path="session/missing.txt")
    with pytest.raises(FileNotFoundError):
        resolver.verify_produced(uri)


def test_file_resolver_verify_produced_raises_when_empty(
    tmp_path: pathlib.Path,
) -> None:
    (tmp_path / "empty.txt").write_bytes(b"")
    resolver = FileArtifactResolver(tmp_path)
    uri = Uri(scheme="file", path="session/empty.txt")
    with pytest.raises(FileNotFoundError):
        resolver.verify_produced(uri)