"""Tests for gremlins.executor.bootstrap."""

from __future__ import annotations

import asyncio
import pathlib
import sys

import pytest
from conftest import MockGremlin

from gremlins.artifacts.registry import ArtifactRegistry
from gremlins.clients.client import Client
from gremlins.executor.bootstrap import run_bootstrap, run_pipeline_bootstrap
from gremlins.executor.state import StateData, build_state
from gremlins.pipeline.bootstrap import Bootstrap, InputSource, InputSources


def test_run_bootstrap_empty_cmds_does_nothing(tmp_path: pathlib.Path) -> None:
    async def _test() -> None:
        await run_bootstrap([], tmp_path)

    asyncio.run(_test())


def test_run_bootstrap_runs_successfully(tmp_path: pathlib.Path) -> None:
    marker = tmp_path / "marker"

    async def _test() -> None:
        await run_bootstrap(
            [
                f"{sys.executable} -c \"import pathlib; pathlib.Path('{marker}').write_text('')\""
            ],
            tmp_path,
        )

    asyncio.run(_test())
    assert marker.exists()


def test_run_bootstrap_non_zero_exit_raises(tmp_path: pathlib.Path) -> None:
    async def _test() -> None:
        with pytest.raises(RuntimeError, match="bootstrap failed"):
            await run_bootstrap(["exit 1"], tmp_path)

    asyncio.run(_test())


def _gremlin(tmp_path: pathlib.Path) -> MockGremlin:
    artifact_dir = tmp_path / "artifacts"
    artifact_dir.mkdir(exist_ok=True)
    registry = ArtifactRegistry(artifact_dir)
    state = build_state(
        data=StateData(),
        client=Client("openai", "gpt-4o"),
        artifact_dir=artifact_dir,
        cwd=str(tmp_path),
        artifacts=registry,
    )
    return MockGremlin(state=state)


def test_launch_cmds_see_source_env(tmp_path: pathlib.Path) -> None:
    marker = tmp_path / "seen.txt"
    bootstrap = Bootstrap(
        source=InputSources(
            {"plan": InputSource(name="plan", types=["string"], optional=True)}
        ),
        launch_cmds=[f'printf "%s" "$plan" > "{marker}"'],
    )
    gremlin = _gremlin(tmp_path)
    assert gremlin.state is not None

    async def _test() -> None:
        await run_pipeline_bootstrap(
            bootstrap,
            cwd=tmp_path,
            artifact_dir=gremlin.state.artifact_dir,
            stage_inputs={"plan": "hello-plan"},
            gremlin=gremlin,
            include_launch=True,
        )

    asyncio.run(_test())
    assert marker.read_text() == "hello-plan"


def test_cli_out_bound_after_launch_cmds(tmp_path: pathlib.Path) -> None:
    artifact_dir = tmp_path / "artifacts"
    artifact_dir.mkdir(exist_ok=True)
    bootstrap = Bootstrap(
        source=InputSources(
            {
                "instructions": InputSource(
                    name="instructions", types=["string"], optional=True
                )
            }
        ),
        launch_cmds=[
            'printf "%s" "$instructions" > "{artifact_dir}/instructions.txt"',
        ],
        cli_out={"instructions?": "file://session/instructions.txt"},
    )
    gremlin = _gremlin(tmp_path)

    async def _test() -> None:
        await run_pipeline_bootstrap(
            bootstrap,
            cwd=tmp_path,
            artifact_dir=gremlin.state.artifact_dir,
            stage_inputs={"instructions": "do the thing"},
            gremlin=gremlin,
            include_launch=True,
        )

    asyncio.run(_test())
    assert gremlin.state.artifacts.produced("instructions")
    assert gremlin.state.artifacts.read("instructions") == "do the thing"


def test_children_only_run_cmds(tmp_path: pathlib.Path) -> None:
    cmds_marker = tmp_path / "cmds"
    launch_marker = tmp_path / "launch"
    bootstrap = Bootstrap(
        cmds=[
            f"{sys.executable} -c \"import pathlib; pathlib.Path('{cmds_marker}').write_text('ok')\""
        ],
        launch_cmds=[
            f"{sys.executable} -c \"import pathlib; pathlib.Path('{launch_marker}').write_text('nope')\""
        ],
        cli_out={"instructions?": "file://session/instructions.txt"},
    )
    gremlin = _gremlin(tmp_path)

    async def _test() -> None:
        await run_pipeline_bootstrap(
            bootstrap,
            cwd=tmp_path,
            artifact_dir=gremlin.state.artifact_dir,
            stage_inputs={"instructions": "do the thing"},
            gremlin=gremlin,
            include_launch=False,
        )

    asyncio.run(_test())
    assert cmds_marker.exists()
    assert not launch_marker.exists()
    assert not gremlin.state.artifacts.produced("instructions")


def test_cli_out_skipped_when_launch_excluded(tmp_path: pathlib.Path) -> None:
    """Resume / children skip launch_cmds and cli_out."""
    artifact_dir = tmp_path / "artifacts"
    artifact_dir.mkdir(exist_ok=True)
    (artifact_dir / "instructions.txt").write_text("stale", encoding="utf-8")
    bootstrap = Bootstrap(
        launch_cmds=[
            'printf "%s" "fresh" > "{artifact_dir}/instructions.txt"',
        ],
        cli_out={"instructions?": "file://session/instructions.txt"},
    )
    gremlin = _gremlin(tmp_path)

    async def _test() -> None:
        await run_pipeline_bootstrap(
            bootstrap,
            cwd=tmp_path,
            artifact_dir=gremlin.state.artifact_dir,
            stage_inputs={"instructions": "fresh"},
            gremlin=gremlin,
            include_launch=False,
        )

    asyncio.run(_test())
    assert not gremlin.state.artifacts.produced("instructions")
    assert (artifact_dir / "instructions.txt").read_text() == "stale"
