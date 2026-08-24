"""Tests for gremlins.executor.bootstrap."""

from __future__ import annotations

import asyncio
import pathlib
import sys

import pytest

from gremlins.executor.bootstrap import run_bootstrap


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
