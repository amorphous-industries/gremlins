from __future__ import annotations

from typing import Any

class RustClient:
    def __init__(
        self,
        provider: str,
        model: str,
        bypass: bool,
        native_block: dict[str, list[str]],
        instructions: str | None = None,
    ) -> None: ...
    @staticmethod
    def cmd(command: str) -> RustClient: ...
    def run(
        self,
        prompt: str,
        label: str,
        model: str | None = None,
        raw_path: str | None = None,
        capture_events: bool = False,
        on_timeout_prompt: str | None = None,
        max_retries: int = 0,
        cwd: str | None = None,
        idle_timeout: float | None = None,
        extra_env: dict[str, str] | None = None,
    ) -> Any: ...
    def resume(self) -> Any: ...
    def reap_all(self) -> None: ...
    @property
    def total_cost_usd(self) -> float | None: ...
