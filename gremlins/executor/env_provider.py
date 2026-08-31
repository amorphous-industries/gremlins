from __future__ import annotations

import pathlib
from collections.abc import Mapping
from typing import NamedTuple, Protocol, runtime_checkable

import gremlins.utils.proc as _proc


class ShellResult(NamedTuple):
    stdout: str
    stderr: str
    returncode: int


@runtime_checkable
class EnvironmentProvider(Protocol):
    """Abstracts shell execution and file I/O for stages."""

    async def run_shell(
        self,
        cmd: str,
        cwd: str,
        env: Mapping[str, str],
        *,
        timeout: float | None = None,
    ) -> ShellResult: ...

    def write_text(self, path: str, content: str) -> None: ...

    def read_text(self, path: str) -> str: ...


class RealEnvironmentProvider:
    """Delegates to real subprocess and filesystem."""

    async def run_shell(
        self,
        cmd: str,
        cwd: str,
        env: Mapping[str, str],
        *,
        timeout: float | None = None,
    ) -> ShellResult:
        result = await _proc.run_shell_async(
            cmd,
            cwd=pathlib.Path(cwd) if cwd else None,
            env=dict(env) if env else None,
            timeout=timeout,
        )
        return ShellResult(result.stdout, result.stderr, result.returncode)

    def write_text(self, path: str, content: str) -> None:
        p = pathlib.Path(path)
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(content, encoding="utf-8")

    def read_text(self, path: str) -> str:
        p = pathlib.Path(path)
        if not p.exists():
            return ""
        return p.read_text(encoding="utf-8")
