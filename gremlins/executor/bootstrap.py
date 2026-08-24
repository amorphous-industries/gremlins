"""Run bootstrap commands in a worktree before pipeline stages begin.

Used at gremlin launch and in parallel child subprocesses so that
every fresh worktree gets its dev environment (venv, etc.) set up.
"""

from __future__ import annotations

import logging
import os
import pathlib

from gremlins.utils import proc

logger = logging.getLogger(__name__)


async def run_bootstrap(cmds: list[str], cwd: pathlib.Path) -> None:
    """Run shell commands in cwd. Non-zero exit raises RuntimeError."""
    if not cmds:
        return
    env = dict(os.environ)
    env["GREMLINS_BOOTSTRAP_CWD"] = str(cwd)
    result = await proc.run_shell_async(" && ".join(cmds), cwd=cwd, env=env)
    if result.returncode != 0:
        err = (result.stderr or result.stdout).strip()
        logger.error("bootstrap failed (exit %d): %s", result.returncode, err[:2000])
        raise RuntimeError(f"bootstrap failed (exit {result.returncode}): {err[:500]}")
    logger.info("bootstrap ok")
