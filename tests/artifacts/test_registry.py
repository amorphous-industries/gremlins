"""Tests for gremlins.artifacts.registry."""

from __future__ import annotations

import json
import pathlib

import pytest
from _gremlins_core.artifacts import Uri

from gremlins.artifacts.registry import (
    ArtifactRegistry,
    DuplicateArtifact,
    MissingArtifact,
)


def make_registry(tmp_path: pathlib.Path) -> ArtifactRegistry:
    return ArtifactRegistry(artifact_dir=tmp_path / "artifacts")


def test_register_resolve(tmp_path: pathlib.Path) -> None:
    r = make_registry(tmp_path)
    uri = Uri.parse("artifact://plan.md")
    r.register(uri)
    resolved = r.resolve(str(uri))
    # register stores the filesystem path; resolve parses it back as a Uri
    assert "plan.md" in resolved.path


def test_resolve_unbound_raises_missing_artifact(tmp_path: pathlib.Path) -> None:
    r = make_registry(tmp_path)
    with pytest.raises(MissingArtifact) as exc_info:
        r.resolve("nope")
    assert exc_info.value.key == "nope"


def test_missing_artifact_is_key_error(tmp_path: pathlib.Path) -> None:
    r = make_registry(tmp_path)
    with pytest.raises(KeyError):
        r.resolve("missing")


def test_produced_true_after_register(tmp_path: pathlib.Path) -> None:
    r = make_registry(tmp_path)
    uri = Uri.parse("artifact://x.md")
    assert not r.exists(str(uri))
    r.register(uri)
    assert r.exists(str(uri))


def test_keys_returns_registered_keys(tmp_path: pathlib.Path) -> None:
    r = make_registry(tmp_path)
    r.register(Uri.parse("artifact://a.md"))
    r.register(Uri.parse("artifact://b.md"))
    assert set(r.keys()) == {"artifact://a.md", "artifact://b.md"}


def test_register_duplicate_no_overwrite_raises(tmp_path: pathlib.Path) -> None:
    r = make_registry(tmp_path)
    uri = Uri.parse("artifact://x.md")
    r.register(uri, overwrite=False)
    with pytest.raises(DuplicateArtifact) as exc_info:
        r.register(uri, overwrite=False)
    assert str(uri) in str(exc_info.value)


def test_read_after_register(tmp_path: pathlib.Path) -> None:
    artifact_dir = tmp_path / "artifacts"
    artifact_dir.mkdir()
    r = ArtifactRegistry(artifact_dir=artifact_dir)
    r.register(Uri.parse("artifact://plan.md"))
    stored = r.read("artifact://plan.md")
    assert isinstance(stored, str)
    assert "plan.md" in stored


def test_registry_path_derives_from_artifact_dir(tmp_path: pathlib.Path) -> None:
    r = ArtifactRegistry(artifact_dir=tmp_path / "artifacts")
    assert r.registry_path == tmp_path / "registry.json"


def test_register_persists_to_file(tmp_path: pathlib.Path) -> None:
    r = ArtifactRegistry(artifact_dir=tmp_path / "artifacts")
    r.register(Uri.parse("artifact://plan.md"))
    data = json.loads(r.registry_path.read_text())
    assert "artifact://plan.md" in data
    assert data["artifact://plan.md"].endswith("plan.md")


def test_init_loads_from_persist_file(tmp_path: pathlib.Path) -> None:
    (tmp_path / "registry.json").write_text(
        json.dumps({"artifact://plan.md": "/tmp/some/plan.md"})
    )
    r = ArtifactRegistry(artifact_dir=tmp_path / "artifacts")
    assert r.resolve("artifact://plan.md").path == "/tmp/some/plan.md"


def test_persist_survives_roundtrip(tmp_path: pathlib.Path) -> None:
    artifact_dir = tmp_path / "artifacts"
    r1 = ArtifactRegistry(artifact_dir=artifact_dir)
    r1.register(Uri.parse("artifact://pr.md"))
    r2 = ArtifactRegistry(artifact_dir=artifact_dir)
    assert r2.exists("artifact://pr.md")
    stored = r2.read("artifact://pr.md")
    assert isinstance(stored, str)
    assert "pr.md" in stored


def test_unbind_removes_binding(tmp_path: pathlib.Path) -> None:
    r = make_registry(tmp_path)
    uri = Uri.parse("artifact://x.md")
    r.register(uri)
    assert r.exists(str(uri))
    r.unbind(str(uri))
    assert not r.exists(str(uri))


def test_unbind_persists_removal(tmp_path: pathlib.Path) -> None:
    r = make_registry(tmp_path)
    uri = Uri.parse("artifact://x.md")
    r.register(uri)
    r.unbind(str(uri))
    data = json.loads(r.registry_path.read_text())
    assert str(uri) not in data


def test_unbind_missing_key_is_noop(tmp_path: pathlib.Path) -> None:
    r = make_registry(tmp_path)
    r.unbind("does-not-exist")  # must not raise


def test_register_still_raises_duplicate_after_unbind_register_no_overwrite(
    tmp_path: pathlib.Path,
) -> None:
    r = make_registry(tmp_path)
    uri = Uri.parse("artifact://x.md")
    r.register(uri, overwrite=False)
    r.unbind(str(uri))
    r.register(uri, overwrite=False)  # clean re-register after unbind
    with pytest.raises(DuplicateArtifact):
        r.register(uri, overwrite=False)  # register(overwrite=False) is still strict


