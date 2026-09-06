"""Tests for dotted-key resolution in resolve_interpolation_map."""

from __future__ import annotations

import asyncio
import pathlib

import pytest
from _gremlins_core.artifacts import Uri
from conftest import MINIMAL_EVENTS, MockGremlin

from gremlins.artifacts.registry import ArtifactRegistry
from gremlins.artifacts.resolve import resolve_interpolation_map
from gremlins.executor.state import StateData, build_state
from gremlins.stages.agent import Agent
from gremlins.stages.exec import Exec
from gremlins.stages.outcome import Done
from tests.fake_client import FakeClient


def _make_registry(tmp_path: pathlib.Path) -> ArtifactRegistry:
    artifact_dir = tmp_path / "artifacts"
    artifact_dir.mkdir(exist_ok=True)
    return ArtifactRegistry(artifact_dir, cwd=tmp_path)


def _make_state(tmp_path: pathlib.Path, client=None):
    artifact_dir = tmp_path / "artifacts"
    artifact_dir.mkdir(exist_ok=True)
    return build_state(
        data=StateData(),
        client=client or FakeClient(),
        artifact_dir=artifact_dir,
        worktree=tmp_path,
    )


# --- resolve_interpolation_map unit tests ---


def test_simple_key_no_dots(tmp_path):
    reg = _make_registry(tmp_path)
    (tmp_path / "artifacts" / "val.txt").write_text("hello")
    reg.bind("key", Uri.parse("file://session/val.txt"))
    result = resolve_interpolation_map(reg, {"VAR": "key"})
    assert result == {"VAR": "hello"}


def test_dotted_key_reads_attribute(tmp_path):
    reg = _make_registry(tmp_path)
    reg.write(
        "pr", {"url": "https://github.com/o/r/pull/7", "number": 7, "branch": "feat-x"}
    )
    result = resolve_interpolation_map(reg, {"branch": "pr.branch"})
    assert result == {"branch": "feat-x"}


def test_dotted_key_number_attribute(tmp_path):
    reg = _make_registry(tmp_path)
    reg.write(
        "pr", {"url": "https://github.com/o/r/pull/42", "number": 42, "branch": "main"}
    )
    result = resolve_interpolation_map(reg, {"num": "pr.number"})
    assert result == {"num": "42"}


def test_dotted_key_url_attribute(tmp_path):
    reg = _make_registry(tmp_path)
    reg.write(
        "pr", {"url": "https://github.com/o/r/pull/3", "number": 3, "branch": "fix"}
    )
    result = resolve_interpolation_map(reg, {"url": "pr.url"})
    assert result == {"url": "https://github.com/o/r/pull/3"}


def test_nested_dotted_path(tmp_path):
    reg = _make_registry(tmp_path)
    reg.write("obj", {"inner": {"value": "deep"}})
    result = resolve_interpolation_map(reg, {"v": "obj.inner.value"})
    assert result == {"v": "deep"}


def test_unknown_attribute_raises(tmp_path):
    reg = _make_registry(tmp_path)
    reg.write(
        "pr", {"url": "https://github.com/o/r/pull/1", "number": 1, "branch": "b"}
    )
    with pytest.raises(ValueError, match="has no key"):
        resolve_interpolation_map(reg, {"x": "pr.nonexistent"})


def test_private_attribute_raises(tmp_path):
    reg = _make_registry(tmp_path)
    reg.write(
        "pr", {"url": "https://github.com/o/r/pull/1", "number": 1, "branch": "b"}
    )
    with pytest.raises(ValueError, match="private attribute"):
        resolve_interpolation_map(reg, {"x": "pr.__class__"})


def test_empty_segment_raises(tmp_path):
    reg = _make_registry(tmp_path)
    with pytest.raises(ValueError, match="empty segment"):
        resolve_interpolation_map(reg, {"x": "pr."})


# --- opaque:// opaque URI returns {"uri": ...} ---


def test_opaque_uri_attribute(tmp_path):
    reg = _make_registry(tmp_path)
    reg.bind("plan", Uri.parse("opaque://issue/42"))
    result = resolve_interpolation_map(reg, {"ref": "plan.uri"})
    assert result == {"ref": "opaque://issue/42"}


# --- exec integration: dotted key becomes env var ---


def test_exec_dotted_key_injects_env_var(tmp_path):
    state = _make_state(tmp_path)
    state.artifacts.write(
        "pr",
        {"url": "https://github.com/o/r/pull/5", "number": 5, "branch": "my-branch"},
    )

    out_file = tmp_path / "branch.txt"
    stage = Exec(
        "push",
        {"cmds": [f'echo "$branch" > {out_file}']},
        interpolation_map={"branch": "pr.branch"},
    )
    gremlin = MockGremlin(state=state)
    result = asyncio.run(stage.run(gremlin))
    assert isinstance(result, Done)
    assert out_file.read_text().strip() == "my-branch"


# --- agent integration: dotted key substituted into prompt ---


def test_agent_dotted_key_substituted_into_prompt(tmp_path):
    client = FakeClient(fixtures={"push-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    state.artifacts.write(
        "pr",
        {"url": "https://github.com/o/r/pull/9", "number": 9, "branch": "agent-branch"},
    )

    agent = Agent(
        "push-agent",
        ["Push to branch: {branch}"],
        {},
        interpolation_map={"branch": "pr.branch"},
    )
    gremlin = MockGremlin(state=state)
    asyncio.run(agent.run(gremlin))

    assert len(client.calls) == 1
    assert "agent-branch" in client.calls[0].prompt
