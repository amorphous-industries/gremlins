"""Run bootstrap commands in a worktree before pipeline stages begin.

Used at gremlin launch and in parallel child subprocesses so that
every fresh worktree gets its dev environment (venv, etc.) set up.
"""

from __future__ import annotations

import logging
import os
import pathlib
from collections.abc import Mapping
from typing import TYPE_CHECKING, Any

from gremlins.pipeline.bootstrap import (
    Bootstrap,
    source_env,
    substitute_bootstrap_vars,
    validate_source_values,
)
from gremlins.utils import proc

if TYPE_CHECKING:
    from gremlins.executor.gremlin import Gremlin

logger = logging.getLogger(__name__)


async def run_bootstrap(
    cmds: list[str],
    cwd: pathlib.Path,
    extra_env: dict[str, str] | None = None,
) -> None:
    """Run shell commands in cwd. Non-zero exit raises RuntimeError."""
    cmds = [c.rstrip() for c in cmds if c.strip()]
    if not cmds:
        return
    env = dict(os.environ)
    env["GREMLINS_BOOTSTRAP_CWD"] = str(cwd)
    if extra_env:
        env.update(extra_env)
    result = await proc.run_shell_async(" && ".join(cmds), cwd=cwd, env=env)
    if result.returncode != 0:
        err = (result.stderr or result.stdout).strip()
        logger.error("bootstrap failed (exit %d): %s", result.returncode, err[:2000])
        raise RuntimeError(f"bootstrap failed (exit {result.returncode}): {err[:500]}")
    logger.info("bootstrap ok")


async def run_pipeline_bootstrap(
    bootstrap: Bootstrap,
    *,
    cwd: pathlib.Path,
    artifact_dir: pathlib.Path,
    stage_inputs: Mapping[str, Any],
    gremlin: Gremlin,
    include_launch: bool,
) -> None:
    """Run worktree cmds, then (main first start only) launch_cmds and cli_out."""
    if bootstrap.cmds:
        await run_bootstrap(bootstrap.cmds, cwd)
    if not include_launch:
        return
    if bootstrap.launch_cmds:
        validate_source_values(bootstrap.source, stage_inputs)
        env = source_env(bootstrap.source, stage_inputs)
        cmds = [
            substitute_bootstrap_vars(c, artifact_dir=artifact_dir, cwd=cwd)
            for c in bootstrap.launch_cmds
        ]
        await run_bootstrap(cmds, cwd, extra_env=env)
    if bootstrap.cli_out:
        from gremlins.stages.exec import Exec

        binder = Exec("bootstrap", {}, out_map=dict(bootstrap.cli_out))
        await binder.run(gremlin)
