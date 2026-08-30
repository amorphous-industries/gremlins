"""Unit tests for the Agent primitive stage."""

from __future__ import annotations

import asyncio
import json
import pathlib
import re
from typing import TYPE_CHECKING, Any, cast

import pytest
from conftest import MINIMAL_EVENTS, MockGremlin

from gremlins.artifacts.registry import ArtifactRegistry, MissingArtifact
from gremlins.artifacts.uri import Uri
from gremlins.executor.state import State, StateData, build_state
from gremlins.stages.agent import Agent
from gremlins.stages.outcome import Done
from tests.fake_client import FakeClient

if TYPE_CHECKING:
    from gremlins.executor.gremlin import Gremlin


def _make_state(
    tmp_path: pathlib.Path,
    client: FakeClient | None = None,
    *,
    registry: ArtifactRegistry | None = None,
) -> State:
    if client is None:
        client = FakeClient(fixtures={"my-agent": MINIMAL_EVENTS})
    reg = registry or ArtifactRegistry(tmp_path / "artifacts", cwd=tmp_path)
    return build_state(
        data=StateData(),
        client=client,
        artifact_dir=tmp_path / "artifacts",
        worktree=tmp_path,
        artifacts=reg,
    )


def _make_agent(
    *,
    prompts: list[str] | None = None,
    interpolation_map: dict[str, str] | None = None,
    bind_map: dict[str, str] | None = None,
    options: dict[str, Any] | None = None,
    name: str = "my-agent",
) -> Agent:
    return Agent(
        name,
        prompts or ["Hello {content}"],
        options or {},
        interpolation_map=interpolation_map,
        bind_map=bind_map,
    )


# --- in: resolution and prompt interpolation ---


def test_in_content_substituted_into_prompt(tmp_path):
    registry = ArtifactRegistry(tmp_path / "artifacts", cwd=tmp_path)
    (tmp_path / "artifacts").mkdir(exist_ok=True)
    (tmp_path / "artifacts" / "plan.md").write_bytes(b"# My Plan")
    registry.bind("plan", Uri.parse("file://session/plan.md"))

    client = FakeClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client, registry=registry)
    agent = _make_agent(
        prompts=["Process: {plan_text}"], interpolation_map={"plan_text": "plan"}
    )

    asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))

    assert len(client.calls) == 1
    assert "# My Plan" in client.calls[0].prompt


def test_missing_in_key_raises_missing_artifact(tmp_path):
    state = _make_state(tmp_path)
    agent = _make_agent(interpolation_map={"content": "unbound-key"})

    with pytest.raises(MissingArtifact):
        asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))


def test_no_interpolation_map_runs_prompt_unchanged(tmp_path):
    client = FakeClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(prompts=["Static prompt"], interpolation_map=None)

    result = asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))

    assert isinstance(result, Done)
    assert client.calls[0].prompt == "Static prompt"


# --- out: verification ---


def test_verify_produced_passes_when_out_file_written(tmp_path):
    class WritingClient(FakeClient):
        async def run(self, prompt, *, label, **kwargs):
            # Extract the slugged path from {out_file} in the prompt.
            m = re.search(r"`([^`]*[0-9a-f]+_output\.md)`", prompt)
            assert m, prompt
            pathlib.Path(m.group(1)).parent.mkdir(parents=True, exist_ok=True)
            pathlib.Path(m.group(1)).write_text("# Output")
            return await super().run(prompt, label=label, **kwargs)

    client = WritingClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(
        prompts=["Write output to `{out_file}`"],
        bind_map={"result": "file://session/output.md"},
    )

    result = asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))

    assert isinstance(result, Done)
    assert state.artifacts is not None
    assert state.artifacts.produced("result")


def test_verify_produced_fails_when_out_file_missing(tmp_path):
    client = FakeClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(
        prompts=["Write output"],
        bind_map={"result": "file://session/missing.md"},
    )

    with pytest.raises(FileNotFoundError):
        asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))


def test_bind_uri_bound_in_registry_before_agent_runs(tmp_path):
    seen_bound_before_run: list[bool] = []

    class CheckingClient(FakeClient):
        async def run(self, prompt, *, label, **kwargs):
            registry = state.artifacts
            seen_bound_before_run.append(
                registry is not None and registry.produced("result")
            )
            # Extract slugged path from {out_file} and write so verify passes.
            m = re.search(r"`([^`]*[0-9a-f]+_output\.md)`", prompt)
            if m:
                p = pathlib.Path(m.group(1))
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_text("# Output", encoding="utf-8")
            return await super().run(prompt, label=label, **kwargs)

    client = CheckingClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(
        prompts=["Write output to `{out_file}`"],
        bind_map={"result": "file://session/output.md"},
    )

    asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))
    assert seen_bound_before_run == [True]


# --- with_dict parsing ---


def test_with_dict_parses_interpolation_and_bind_maps(tmp_path):
    d = {
        "name": "my-agent",
        "type": "agent",
        "prompt": ["Do {task}"],
        "interpolation": {"task": "artifact.task-key"},
        "bind": {"artifact.result": "file://session/result.md"},
    }
    agent = Agent.with_dict(d)
    assert agent.interpolation_map == {"task": "task-key"}
    assert agent.bind_map == {"result": "file://session/result.md"}


def test_with_dict_rejects_non_dict_interpolation(tmp_path):
    d = {"name": "x", "type": "agent", "interpolation": "not-a-dict"}
    with pytest.raises(ValueError, match="'interpolation' must be a mapping"):
        Agent.with_dict(d)


def test_with_dict_rejects_non_dict_bind(tmp_path):
    d = {"name": "x", "type": "agent", "bind": ["list"]}
    with pytest.raises(ValueError, match="'bind' must be a mapping"):
        Agent.with_dict(d)


def test_with_dict_rejects_old_in_key(tmp_path):
    d = {"name": "x", "type": "agent", "in": {"task": "key"}}
    with pytest.raises(ValueError, match="'in'/'out' keys are no longer supported"):
        Agent.with_dict(d)


def test_with_dict_rejects_old_out_key(tmp_path):
    d = {"name": "x", "type": "agent", "out": {"key": "uri"}}
    with pytest.raises(ValueError, match="'in'/'out' keys are no longer supported"):
        Agent.with_dict(d)


# --- registry always required ---


# --- single-file out: auto-management ---


def test_single_file_out_prompt_gets_slug_prefixed_name(tmp_path):
    class WritingClient(FakeClient):
        async def run(self, prompt, *, label, cwd=None, **kwargs):
            m = re.search(r"`([^`]*[0-9a-f]+_output\.md)`", prompt)
            assert m, prompt
            pathlib.Path(m.group(1)).parent.mkdir(parents=True, exist_ok=True)
            pathlib.Path(m.group(1)).write_text("# Output", encoding="utf-8")
            return await super().run(prompt, label=label, cwd=cwd, **kwargs)

    client = WritingClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(
        prompts=["Write to `{out_file}`"],
        bind_map={"result": "file://session/output.md"},
    )

    asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))

    assert len(client.calls) == 1
    m = re.fullmatch(
        r"Write to `"
        + re.escape(str(state.artifact_dir))
        + r"/([0-9a-f]{8})_output\.md`",
        client.calls[0].prompt,
    )
    assert m, client.calls[0].prompt


def test_single_file_out_keeps_slug_in_artifact_dir(tmp_path):
    """Slugged file remains on disk after the agent completes — the slug is
    never stripped, giving each run a unique file footprint."""

    class WritingClient(FakeClient):
        async def run(self, prompt, *, label, cwd=None, **kwargs):
            m = re.search(r"`([^`]*[0-9a-f]+_output\.md)`", prompt)
            assert m, prompt
            pathlib.Path(m.group(1)).parent.mkdir(parents=True, exist_ok=True)
            pathlib.Path(m.group(1)).write_text("# Output", encoding="utf-8")
            return await super().run(prompt, label=label, cwd=cwd, **kwargs)

    client = WritingClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(
        prompts=["Write to `{out_file}`"],
        bind_map={"result": "file://session/output.md"},
    )

    result = asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))
    assert isinstance(result, Done)
    # The slugged file is what the agent was told to write — it stays on disk.
    slugged = list(state.artifact_dir.glob("*_output.md"))
    assert len(slugged) == 1, f"expected 1 slugged output.md, got {slugged}"
    assert slugged[0].read_text(encoding="utf-8") == "# Output"
    # No unslugged output.md (slug is not stripped).
    assert not (state.artifact_dir / "output.md").exists()


def test_single_file_out_uses_substituted_out_map_name(tmp_path):
    class WritingClient(FakeClient):
        async def run(self, prompt, *, label, cwd=None, **kwargs):
            m = re.search(r"`([^`]*[0-9a-f]+_review-code\.md)`", prompt)
            assert m, prompt
            pathlib.Path(m.group(1)).parent.mkdir(parents=True, exist_ok=True)
            pathlib.Path(m.group(1)).write_text("# Review", encoding="utf-8")
            return await super().run(prompt, label=label, cwd=cwd, **kwargs)

    client = WritingClient(fixtures={"review-code": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(
        name="review-code",
        prompts=["Write review to `{out_file}`"],
        bind_map={"{name}": "file://session/{name}.md"},
    )

    result = asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))
    assert isinstance(result, Done)
    slugged = list(state.artifact_dir.glob("*_review-code.md"))
    assert len(slugged) == 1
    assert slugged[0].read_text(encoding="utf-8") == "# Review"


def test_single_file_out_missing_source_raises(tmp_path):
    client = FakeClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(
        prompts=["Write nothing"],
        bind_map={"result": "file://session/never-written.md"},
    )

    with pytest.raises(FileNotFoundError):
        asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))


# --- multi-file out: auto-management (best-effort) ---


def test_multi_file_out_prompt_gets_json_mapping(tmp_path):
    client = FakeClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(
        prompts=["Write to {out_files}"],
        bind_map={
            "blah": "file://session/blah.md",
            "foo": "file://session/foo.md",
            "biz": "file://session/biz.md",
        },
    )

    asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))

    assert len(client.calls) == 1
    m = re.fullmatch(r"Write to (\{.*\})", client.calls[0].prompt)
    assert m, client.calls[0].prompt
    mapping = json.loads(m.group(1))
    assert set(mapping) == {"blah.md", "foo.md", "biz.md"}
    ad = str(state.artifact_dir)
    for name, actual in mapping.items():
        assert re.fullmatch(rf"{re.escape(ad)}/[0-9a-f]{{8}}_{re.escape(name)}", actual)


def test_multi_file_out_renames_only_written_files(tmp_path):
    class WritingClient(FakeClient):
        async def run(self, prompt, *, label, cwd=None, **kwargs):
            # Write two of the three declared files; biz.md is skipped.
            m = re.search(r"\{.*\}", prompt)
            mapping = json.loads(m.group(0))
            for name in ("blah.md", "foo.md"):
                p = pathlib.Path(mapping[name])
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_text(f"# {name}", encoding="utf-8")
            return await super().run(prompt, label=label, cwd=cwd, **kwargs)

    client = WritingClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(
        prompts=["Write to {out_files}"],
        bind_map={
            "blah": "file://session/blah.md",
            "foo": "file://session/foo.md",
            "biz": "file://session/biz.md",
        },
    )

    result = asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))
    assert isinstance(result, Done)
    # Files stay at slugged paths — the slug is never stripped.
    slugged_blah = list(state.artifact_dir.glob("*_blah.md"))
    slugged_foo = list(state.artifact_dir.glob("*_foo.md"))
    assert len(slugged_blah) == 1
    assert len(slugged_foo) == 1
    assert slugged_blah[0].read_text(encoding="utf-8") == "# blah.md"
    assert slugged_foo[0].read_text(encoding="utf-8") == "# foo.md"
    assert not (state.artifact_dir / "blah.md").exists()
    assert not (state.artifact_dir / "biz.md").exists()


def test_multi_file_out_missing_files_do_not_raise(tmp_path):
    client = FakeClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(
        prompts=["Write nothing"],
        bind_map={
            "blah": "file://session/blah.md",
            "foo": "file://session/foo.md",
        },
    )

    result = asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))
    assert isinstance(result, Done)
