from __future__ import annotations

from collections.abc import Callable
from typing import Any

class PyUsageStats:
    prompt_tokens: int
    completion_tokens: int
    cached_input_tokens: int
    cache_creation_input_tokens: int
    reasoning_tokens: int
    turns: int
    def __init__(
        self,
        prompt_tokens: int = 0,
        completion_tokens: int = 0,
        cached_input_tokens: int = 0,
        cache_creation_input_tokens: int = 0,
        reasoning_tokens: int = 0,
        turns: int = 0,
    ) -> None: ...

class PyCompletedRun:
    exit_code: int
    text_result: str | None
    events: list[str] | None
    cost_usd: float | None
    token_usage: PyUsageStats | None
    def __init__(
        self,
        exit_code: int = 0,
        text_result: str | None = None,
        events: list[str] | None = None,
        cost_usd: float | None = None,
        token_usage: PyUsageStats | None = None,
    ) -> None: ...

class RustClient:
    provider: str
    model: str
    extra_params: dict[str, str]
    def __init__(
        self,
        provider: str,
        model: str,
        native_block: dict[str, list[str]] | None = None,
        extra_params: dict[str, str] | None = None,
    ) -> None: ...
    @staticmethod
    def cmd(command: str) -> RustClient: ...
    @staticmethod
    def parse(s: str) -> RustClient: ...
    @staticmethod
    def from_spec(s: str) -> RustClient: ...
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
        artifact_dir: str | None = None,
        idle_timeout: float | None = None,
        extra_env: dict[str, str] | None = None,
        expected_artifact_paths: list[str] | None = None,
        artifact_reminder_count: int = 0,
    ) -> Any: ...
    def resume(self) -> Any: ...
    def reap_all(self) -> None: ...
    @property
    def total_cost_usd(self) -> float | None: ...

CLIENT_FACTORIES: dict[str, Callable[..., RustClient]]
_DEFAULT_ALLOWED_TOOLS: list[str]
