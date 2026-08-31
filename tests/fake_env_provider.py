from __future__ import annotations

from collections.abc import Mapping

from gremlins.executor.env_provider import ShellResult


class FakeEnvironmentProvider:
    """Records shell/file calls for test assertion."""

    def __init__(self) -> None:
        self.shell_calls: list[tuple[str, str, dict[str, str], float | None]] = []
        self.write_calls: list[tuple[str, str]] = []
        self.read_calls: list[str] = []

    async def run_shell(
        self,
        cmd: str,
        cwd: str,
        env: Mapping[str, str],
        *,
        timeout: float | None = None,
    ) -> ShellResult:
        self.shell_calls.append((cmd, cwd, dict(env), timeout))
        return ShellResult("", "", 0)

    def write_text(self, path: str, content: str) -> None:
        self.write_calls.append((path, content))

    def read_text(self, path: str) -> str:
        self.read_calls.append(path)
        return ""
