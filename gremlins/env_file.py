from __future__ import annotations

import os
import pathlib
import subprocess

# Variables set by bash itself that are not meaningful to propagate.
_BASH_INTERNALS = frozenset(
    {
        "_",
        "BASH",
        "BASH_VERSION",
        "BASH_VERSINFO",
        "BASHOPTS",
        "BASHPID",
        "PPID",
        "SHLVL",
        "SHELLOPTS",
    }
)


def load_env_file(
    path: pathlib.Path, cwd: pathlib.Path | None = None
) -> dict[str, str]:
    before = dict(os.environ)
    # Strip BASH_ENV so bash doesn't auto-source an unrelated file.
    env = {k: v for k, v in os.environ.items() if k != "BASH_ENV"}
    try:
        result = subprocess.run(
            ["bash", "-c", 'source "$1" >/dev/null && env -0', "--", str(path)],
            capture_output=True,
            check=False,
            env=env,
            cwd=cwd,
        )
    except FileNotFoundError:
        raise RuntimeError(f"failed to source {path}: bash not found")
    if result.returncode != 0:
        raise RuntimeError(
            f"failed to source {path}:\n{result.stderr.decode(errors='replace').strip()}"
        )
    after: dict[str, str] = {}
    for entry in result.stdout.split(b"\0"):
        decoded = entry.decode(errors="replace")
        if "=" in decoded:
            k, _, v = decoded.partition("=")
            after[k] = v
    return {
        k: v
        for k, v in after.items()
        if before.get(k) != v and k not in _BASH_INTERNALS
    }


def load_env_file_isolated(
    path: pathlib.Path,
    *,
    base_env: dict[str, str],
    cwd: pathlib.Path | None = None,
) -> dict[str, str]:
    """Source `path` in a clean bash environment and return every env var.

    `base_env` is the **only** environment the bash subprocess receives.
    It must contain PATH, HOME, and all system vars, and nothing else.
    The caller is responsible for constructing `base_env` correctly.
    """
    try:
        result = subprocess.run(
            ["bash", "-c", 'source "$1" >/dev/null && env -0', "--", str(path)],
            capture_output=True,
            check=False,
            env=base_env,
            cwd=cwd,
        )
    except FileNotFoundError:
        raise RuntimeError(f"failed to source {path}: bash not found")
    if result.returncode != 0:
        raise RuntimeError(
            f"failed to source {path}:\n{result.stderr.decode(errors='replace').strip()}"
        )
    env: dict[str, str] = {}
    for entry in result.stdout.split(b"\0"):
        decoded = entry.decode(errors="replace")
        if "=" in decoded:
            k, _, v = decoded.partition("=")
            env[k] = v
    # Strip bash internal vars.
    for key in _BASH_INTERNALS:
        env.pop(key, None)
    return env
