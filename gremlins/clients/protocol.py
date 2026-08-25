from __future__ import annotations

from dataclasses import dataclass
from typing import Any


@dataclass
class UsageStats:
    prompt_tokens: int = 0
    completion_tokens: int = 0
    cached_input_tokens: int = 0
    cache_creation_input_tokens: int = 0
    reasoning_tokens: int = 0
    turns: int = 0


@dataclass
class CompletedRun:
    exit_code: int
    text_result: str | None = None
    events: list[dict[str, Any]] | None = None
    cost_usd: float | None = None
    token_usage: UsageStats | None = None
