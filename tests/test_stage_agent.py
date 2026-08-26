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
    in_map: dict[str, str] | None = None,
    out_map: dict[str, str] | None = None,
    options: dict[str, Any] | None = None,
    name: str = "my-agent",
) -> Agent:
    return Agent(
        name,
        prompts or ["Hello {content}"],
        options or {},
        in_map=in_map,
        out_map=out_map,
    )


# --- in: resolution and prompt interpolation ---


def test_in_content_substituted_into_prompt(tmp_path):
    registry = ArtifactRegistry(tmp_path / "artifacts", cwd=tmp_path)
    (tmp_path / "artifacts").mkdir(exist_ok=True)
    (tmp_path / "artifacts" / "plan.md").write_bytes(b"# My Plan")
    registry.bind("plan", Uri.parse("file://session/plan.md"))

    client = FakeClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client, registry=registry)
    agent = _make_agent(prompts=["Process: {plan_text}"], in_map={"plan_text": "plan"})

    asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))

    assert len(client.calls) == 1
    assert "# My Plan" in client.calls[0].prompt


def test_missing_in_key_raises_missing_artifact(tmp_path):
    state = _make_state(tmp_path)
    agent = _make_agent(in_map={"content": "unbound-key"})

    with pytest.raises(MissingArtifact):
        asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))


def test_no_in_map_runs_prompt_unchanged(tmp_path):
    client = FakeClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(prompts=["Static prompt"], in_map=None)

    result = asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))

    assert isinstance(result, Done)
    assert client.calls[0].prompt == "Static prompt"


# --- out: verification ---


def test_verify_produced_passes_when_out_file_written(tmp_path):
    output_file = tmp_path / "artifacts" / "output.md"
    output_file.parent.mkdir(exist_ok=True)

    class WritingClient(FakeClient):
        async def run(self, prompt, *, label, **kwargs):
            output_file.write_text("# Output")
            return await super().run(prompt, label=label, **kwargs)

    client = WritingClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(
        prompts=["Write output"],
        out_map={"result": "file://session/output.md"},
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
        out_map={"result": "file://session/missing.md"},
    )

    with pytest.raises(FileNotFoundError):
        asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))


def test_out_uri_bound_in_registry_before_agent_runs(tmp_path):
    output_file = tmp_path / "artifacts" / "output.md"
    output_file.parent.mkdir(exist_ok=True)
    seen_bound_before_run: list[bool] = []

    class CheckingClient(FakeClient):
        async def run(self, prompt, *, label, **kwargs):
            # Check that the out: key is bound before the agent runs
            registry = state.artifacts
            seen_bound_before_run.append(
                registry is not None and registry.produced("result")
            )
            output_file.write_text("# Output")
            return await super().run(prompt, label=label, **kwargs)

    client = CheckingClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(
        out_map={"result": "file://session/output.md"},
    )

    asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))
    assert seen_bound_before_run == [True]


# --- with_dict parsing ---


def test_with_dict_parses_in_and_out_maps(tmp_path):
    d = {
        "name": "my-agent",
        "type": "agent",
        "prompt": ["Do {task}"],
        "in": {"task": "task-key"},
        "out": {"result": "file://session/result.md"},
    }
    agent = Agent.with_dict(d)
    assert agent.in_map == {"task": "task-key"}
    assert agent.out_map == {"result": "file://session/result.md"}


def test_with_dict_rejects_non_dict_in(tmp_path):
    d = {"name": "x", "type": "agent", "in": "not-a-dict"}
    with pytest.raises(ValueError, match="'in' must be a mapping"):
        Agent.with_dict(d)


def test_with_dict_rejects_non_dict_out(tmp_path):
    d = {"name": "x", "type": "agent", "out": ["list"]}
    with pytest.raises(ValueError, match="'out' must be a mapping"):
        Agent.with_dict(d)


# --- registry always required ---


# --- single-file out: auto-management ---


def test_single_file_out_prompt_gets_slug_prefixed_name(tmp_path):
    class WritingClient(FakeClient):
        async def run(self, prompt, *, label, cwd=None, **kwargs):
            m = re.search(r"`(\S*[0-9a-f]+_output\.md)`", prompt)
            assert m, prompt
            pathlib.Path(m.group(1)).parent.mkdir(parents=True, exist_ok=True)
            pathlib.Path(m.group(1)).write_text("# Output", encoding="utf-8")
            return await super().run(prompt, label=label, cwd=cwd, **kwargs)

    client = WritingClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(
        prompts=["Write to `{out_file}`"],
        out_map={"result": "file://session/output.md"},
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


def test_single_file_out_strips_slug_in_artifact_dir(tmp_path):
    class WritingClient(FakeClient):
        async def run(self, prompt, *, label, cwd=None, **kwargs):
            m = re.search(r"`(\S*[0-9a-f]+_output\.md)`", prompt)
            assert m, prompt
            pathlib.Path(m.group(1)).parent.mkdir(parents=True, exist_ok=True)
            pathlib.Path(m.group(1)).write_text("# Output", encoding="utf-8")
            return await super().run(prompt, label=label, cwd=cwd, **kwargs)

    client = WritingClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(
        prompts=["Write to `{out_file}`"],
        out_map={"result": "file://session/output.md"},
    )

    result = asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))
    assert isinstance(result, Done)
    assert (state.artifact_dir / "output.md").read_text(encoding="utf-8") == "# Output"
    # After the rename, only the final name exists — no slugged file remains.
    assert list(state.artifact_dir.glob("*_output.md")) == []


def test_single_file_out_uses_substituted_out_map_name(tmp_path):
    class WritingClient(FakeClient):
        async def run(self, prompt, *, label, cwd=None, **kwargs):
            m = re.search(r"`(\S*[0-9a-f]+_review-code\.md)`", prompt)
            assert m, prompt
            pathlib.Path(m.group(1)).parent.mkdir(parents=True, exist_ok=True)
            pathlib.Path(m.group(1)).write_text("# Review", encoding="utf-8")
            return await super().run(prompt, label=label, cwd=cwd, **kwargs)

    client = WritingClient(fixtures={"review-code": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(
        name="review-code",
        prompts=["Write review to `{out_file}`"],
        out_map={"{name}": "file://session/{name}.md"},
    )

    result = asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))
    assert isinstance(result, Done)
    assert (state.artifact_dir / "review-code.md").read_text(
        encoding="utf-8"
    ) == "# Review"


def test_single_file_out_missing_source_raises(tmp_path):
    client = FakeClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(
        prompts=["Write nothing"],
        out_map={"result": "file://session/never-written.md"},
    )

    with pytest.raises(FileNotFoundError):
        asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))


# --- multi-file out: auto-management (best-effort) ---


def test_multi_file_out_prompt_gets_json_mapping(tmp_path):
    client = FakeClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(
        prompts=["Write to {out_files}"],
        out_map={
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
        out_map={
            "blah": "file://session/blah.md",
            "foo": "file://session/foo.md",
            "biz": "file://session/biz.md",
        },
    )

    result = asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))
    assert isinstance(result, Done)
    assert (state.artifact_dir / "blah.md").read_text(encoding="utf-8") == "# blah.md"
    assert (state.artifact_dir / "foo.md").read_text(encoding="utf-8") == "# foo.md"
    assert not (state.artifact_dir / "biz.md").exists()


def test_multi_file_out_missing_files_do_not_raise(tmp_path):
    client = FakeClient(fixtures={"my-agent": MINIMAL_EVENTS})
    state = _make_state(tmp_path, client)
    agent = _make_agent(
        prompts=["Write nothing"],
        out_map={
            "blah": "file://session/blah.md",
            "foo": "file://session/foo.md",
        },
    )

    result = asyncio.run(agent.run(cast("Gremlin", MockGremlin(state))))
    assert isinstance(result, Done)
