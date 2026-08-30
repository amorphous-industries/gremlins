"""Tests for gremlins.stages.exec.Exec."""

from __future__ import annotations

import asyncio
import pathlib

import pytest
from conftest import MockGremlin

from gremlins.artifacts.registry import ArtifactRegistry, MissingArtifact
from gremlins.artifacts.uri import Uri
from gremlins.executor.state import StateData, build_state
from gremlins.stages.exec import Exec
from gremlins.stages.outcome import Bail, Done
from tests.fake_client import FakeClient


def _make_state(tmp_path: pathlib.Path, **kw):
    kw.setdefault("worktree", tmp_path)
    artifact_dir = tmp_path / "artifacts"
    artifact_dir.mkdir(exist_ok=True)
    return build_state(
        data=StateData(),
        client=FakeClient(),
        artifact_dir=artifact_dir,
        **kw,
    )


def _exec(name: str = "test", cmds=None, *, interpolation_map=None, bind_map=None, timeout=None):
    options = {}
    if cmds is not None:
        options["cmds"] = cmds
    if timeout is not None:
        options["timeout"] = timeout
    return Exec(name, options, interpolation_map=interpolation_map, bind_map=bind_map)


# ---------------------------------------------------------------------------
# Happy path — no in/out
# ---------------------------------------------------------------------------


def test_no_in_out_returns_done(tmp_path):
    state = _make_state(tmp_path)
    stage = _exec(cmds=["true"])
    result = asyncio.run(stage.run(MockGremlin(state=state)))
    assert isinstance(result, Done)


def test_no_cmds_returns_done(tmp_path):
    state = _make_state(tmp_path)
    stage = _exec(cmds=[])
    result = asyncio.run(stage.run(MockGremlin(state=state)))
    assert isinstance(result, Done)


# ---------------------------------------------------------------------------
# in: artifact injection
# ---------------------------------------------------------------------------


def test_interpolation_map_injects_env_var(tmp_path):
    state = _make_state(tmp_path)
    (state.artifact_dir / "value.txt").write_text("hello")
    state.artifacts.bind("my-key", Uri.parse("file://session/value.txt"))

    out_file = tmp_path / "captured.txt"
    stage = _exec(
        cmds=[f'echo "$MY_VAR" > {out_file}'],
        interpolation_map={"MY_VAR": "my-key"},
    )
    asyncio.run(stage.run(MockGremlin(state=state)))
    assert out_file.read_text().strip() == "hello"


def test_interpolation_map_missing_artifact_raises(tmp_path):
    state = _make_state(tmp_path)
    stage = _exec(cmds=["true"], interpolation_map={"X": "not-bound"})
    with pytest.raises(MissingArtifact):
        asyncio.run(stage.run(MockGremlin(state=state)))


# ---------------------------------------------------------------------------
# out: file://session/<name>
# ---------------------------------------------------------------------------


def test_bind_file_scheme_binds_and_verifies(tmp_path):
    state = _make_state(tmp_path)
    (state.artifact_dir / "out.txt").write_text("data")
    stage = _exec(cmds=["true"], bind_map={"result": "file://session/out.txt"})
    result = asyncio.run(stage.run(MockGremlin(state=state)))
    assert isinstance(result, Done)
    assert state.artifacts.produced("result")
    assert state.artifacts.resolve("result") == Uri.parse("file://session/out.txt")


def test_bind_file_scheme_missing_file_raises(tmp_path):
    state = _make_state(tmp_path)
    stage = _exec(cmds=["true"], bind_map={"result": "file://session/missing.txt"})
    with pytest.raises(FileNotFoundError):
        asyncio.run(stage.run(MockGremlin(state=state)))


# ---------------------------------------------------------------------------
# out: git://range
# ---------------------------------------------------------------------------


def test_bind_git_range_binds_commit_range(tmp_path, monkeypatch):
    state = _make_state(tmp_path)
    monkeypatch.setattr(
        "gremlins.stages.exec.snapshot_head_before", lambda cwd=None: "abc123"
    )
    monkeypatch.setattr(
        "gremlins.artifacts.registry.git_utils.head_sha", lambda cwd=None: "def456"
    )
    stage = _exec(cmds=["true"], bind_map={"commits": "git://range"})
    result = asyncio.run(stage.run(MockGremlin(state=state)))
    assert isinstance(result, Done)
    assert state.artifacts.produced("commits")
    bound_uri = state.artifacts.resolve("commits")
    assert str(bound_uri) == "git://range/abc123..def456"


def test_bind_git_range_empty_diff_still_binds(tmp_path, monkeypatch):
    state = _make_state(tmp_path)
    monkeypatch.setattr(
        "gremlins.stages.exec.snapshot_head_before", lambda cwd=None: "abc123"
    )
    # HEAD doesn't advance — same SHA before and after
    monkeypatch.setattr(
        "gremlins.artifacts.registry.git_utils.head_sha", lambda cwd=None: "abc123"
    )
    stage = _exec(cmds=["true"], bind_map={"commits": "git://range"})
    result = asyncio.run(stage.run(MockGremlin(state=state)))
    assert isinstance(result, Done)
    assert state.artifacts.produced("commits")


# ---------------------------------------------------------------------------
# out: {read:KEY} URI substitution
# ---------------------------------------------------------------------------


def test_bind_read_sub_resolves_uri(tmp_path):
    state = _make_state(tmp_path)
    foo_file = state.artifact_dir / "foo.txt"
    stage = _exec(
        cmds=[f'echo 42 > "{foo_file}"'],
        bind_map={
            "foo": "file://session/foo.txt",
            "bar": "gh://pr/{read:foo}",
        },
    )
    result = asyncio.run(stage.run(MockGremlin(state=state)))
    assert isinstance(result, Done)
    assert state.artifacts.resolve("bar") == Uri.parse("gh://pr/42")


def test_bind_read_sub_backward_ref_raises(tmp_path):
    state = _make_state(tmp_path)
    stage = _exec(
        cmds=["true"],
        bind_map={
            "bar": "gh://pr/{read:foo}",
            "foo": "file://session/foo.txt",
        },
    )
    with pytest.raises(MissingArtifact):
        asyncio.run(stage.run(MockGremlin(state=state)))


def test_bind_read_sub_unbound_key_raises(tmp_path):
    state = _make_state(tmp_path)
    stage = _exec(cmds=["true"], bind_map={"bar": "gh://pr/{read:nonexistent}"})
    with pytest.raises(MissingArtifact):
        asyncio.run(stage.run(MockGremlin(state=state)))


def test_artifact_sub_replaces_with_filesystem_path(tmp_path):
    state = _make_state(tmp_path)
    state.artifacts.bind("my-file", Uri.parse("file://session/my-file.txt"))
    (state.artifact_dir / "my-file.txt").write_text("hello")
    stage = _exec(
        cmds=['test "$(cat {artifact:my-file})" = "hello"'],
    )
    result = asyncio.run(stage.run(MockGremlin(state=state)))
    assert isinstance(result, Done)


def test_artifact_sub_raises_for_unbound_key(tmp_path):
    state = _make_state(tmp_path)
    stage = _exec(cmds=["echo {artifact:nonexistent}"])
    with pytest.raises(MissingArtifact):
        asyncio.run(stage.run(MockGremlin(state=state)))


def test_registry_path_for_resolves_file_session(tmp_path):
    registry = ArtifactRegistry(tmp_path / "artifacts", cwd=tmp_path)
    registry.bind("x", Uri.parse("file://session/foo.txt"))
    path = registry.path_for("x")
    assert path is not None
    assert path.name == "foo.txt"
    assert path.is_absolute()


def test_registry_path_for_returns_none_for_non_file_uri(tmp_path):
    registry = ArtifactRegistry(tmp_path / "artifacts", cwd=tmp_path)
    registry.bind("x", Uri.parse("gh://pr/42"))
    assert registry.path_for("x") is None


def test_registry_path_for_returns_none_for_unbound_key(tmp_path):
    registry = ArtifactRegistry(tmp_path / "artifacts", cwd=tmp_path)
    assert registry.path_for("nonexistent") is None


# ---------------------------------------------------------------------------
# Non-zero exit
# ---------------------------------------------------------------------------


def test_nonzero_exit_raises_bail(tmp_path):
    state = _make_state(tmp_path)
    stage = _exec(cmds=["exit 1"])
    with pytest.raises(Bail):
        asyncio.run(stage.run(MockGremlin(state=state)))


def test_nonzero_exit_writes_log(tmp_path):
    state = _make_state(tmp_path)
    stage = _exec("myname", cmds=["echo oops; exit 1"])
    with pytest.raises(Bail):
        asyncio.run(stage.run(MockGremlin(state=state)))
    assert (state.artifact_dir / "exec-myname.log").exists()


def test_success_writes_log(tmp_path):
    state = _make_state(tmp_path)
    stage = _exec("myname", cmds=["echo hello"])
    result = asyncio.run(stage.run(MockGremlin(state=state)))
    assert isinstance(result, Done)
    assert (state.artifact_dir / "exec-myname.log").exists()


# ---------------------------------------------------------------------------
# timeout option
# ---------------------------------------------------------------------------


def test_timeout_raises_bail(tmp_path):
    state = _make_state(tmp_path)
    stage = _exec(cmds=["sleep 10"], timeout=0.05)
    with pytest.raises(Bail):
        asyncio.run(stage.run(MockGremlin(state=state)))


# ---------------------------------------------------------------------------
# bail artifact
# ---------------------------------------------------------------------------


def test_bail_artifact_on_exit_2(tmp_path):
    """Exit code 2 with bail in bind_map writes the bail artifact."""
    state = _make_state(tmp_path)
    bail_file = state.artifact_dir / "bail"
    bail_file.write_text("something broke")
    stage = _exec(
        cmds=["exit 2"],
        bind_map={"bail": "file://session/bail"},
    )
    result = asyncio.run(stage.run(MockGremlin(state=state)))
    assert isinstance(result, Done)
    assert state.artifacts.produced("bail")
