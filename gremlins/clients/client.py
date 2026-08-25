from __future__ import annotations

import pathlib
import re
from typing import Any

from gremlins.clients.protocol import CompletedRun
from gremlins.clients.registry import CLIENT_FACTORIES

# Matches a trailing :k1=v1,k2=v2,... params suffix.
# Each param is an identifier key followed by = and a value (non-comma, non-equals).
_PARAMS_RE = re.compile(
    r":"
    r"([a-zA-Z_][a-zA-Z0-9_]*=[^=,]+)"
    r"(?:,([a-zA-Z_][a-zA-Z0-9_]*=[^=,]+))*$"
)


def _parse_params(params_str: str) -> dict[str, str]:
    result: dict[str, str] = {}
    for pair in params_str.split(","):
        k, _, v = pair.partition("=")
        if k in result:
            raise ValueError(f"duplicate key {k!r} in client params {params_str!r}")
        result[k] = v
    return result


class Client:
    def __init__(
        self,
        provider: str,
        model: str,
        extra_params: dict[str, str] | None = None,
    ) -> None:
        self.provider = provider
        self.model = model
        self.extra_params: dict[str, str] = dict(extra_params) if extra_params else {}
        self._impl: Any = None

    @classmethod
    def parse(cls, s: str) -> Client:
        if ":" not in s:
            raise ValueError(
                f"invalid client specifier {s!r}: expected 'provider:model'"
            )
        provider, _, rest = s.partition(":")
        if not provider:
            raise ValueError(
                f"invalid client specifier {s!r}: provider must not be empty"
            )
        if not rest:
            raise ValueError(f"invalid client specifier {s!r}: model must not be empty")
        if provider not in CLIENT_FACTORIES:
            raise ValueError(f"unknown provider {provider!r} in client specifier {s!r}")

        extra_params: dict[str, str] = {}
        # cmd provider: the "model" is the full command, no params suffix.
        # Note: param values are matched greedily by _PARAMS_RE (value class is
        # [^=,]+). This means colons inside values are allowed, e.g.
        # "model:reasoning=val:with:colons" parses "val:with:colons" as the
        # reasoning value. This is intentional to support colon-bearing values,
        # but it means a trailing ":suffix" is swallowed into the last param.
        # Models with colons in their name work fine as long as params follow
        # after the model name colon.
        if provider != "cmd":
            m = _PARAMS_RE.search(rest)
            if m:
                extra_params = _parse_params(m.group(0)[1:])  # strip leading :
                rest = rest[: m.start()]
        model = rest
        if not model:
            raise ValueError(f"invalid client specifier {s!r}: model must not be empty")
        return cls(provider=provider, model=model, extra_params=extra_params)

    def __str__(self) -> str:
        s = f"{self.provider}:{self.model}"
        if self.extra_params:
            params = ",".join(f"{k}={v}" for k, v in self.extra_params.items())
            s += f":{params}"
        return s

    def __repr__(self) -> str:
        base = f"Client({self.provider!r}, {self.model!r}"
        if self.extra_params:
            base += f", extra_params={self.extra_params!r}"
        return base + ")"

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, Client):
            return NotImplemented
        return (
            self.provider == other.provider
            and self.model == other.model
            and self.extra_params == other.extra_params
        )

    def __hash__(self) -> int:
        return hash(
            (self.provider, self.model, tuple(sorted(self.extra_params.items())))
        )

    def _get_impl(self) -> Any:
        if self._impl is None:
            if self.provider not in CLIENT_FACTORIES:
                raise ValueError(f"unknown provider {self.provider!r}")
            self._impl = CLIENT_FACTORIES[self.provider](self.model, self.extra_params)
        return self._impl

    async def run(
        self,
        prompt: str,
        *,
        label: str,
        model: str | None = None,
        raw_path: pathlib.Path | None = None,
        capture_events: bool = False,
        on_timeout_prompt: str | None = None,
        max_retries: int = 3,
        cwd: pathlib.Path | None = None,
        idle_timeout: float | None = None,
        extra_env: dict[str, str] | None = None,
    ) -> CompletedRun:
        return await self._get_impl().run(
            prompt,
            label=label,
            model=model if model is not None else self.model,
            raw_path=raw_path,
            capture_events=capture_events,
            on_timeout_prompt=on_timeout_prompt,
            max_retries=max_retries,
            cwd=cwd,
            idle_timeout=idle_timeout,
            extra_env=extra_env,
        )

    async def resume(self) -> CompletedRun:
        return await self._get_impl().resume()

    def reap_all(self) -> None:
        if self._impl is not None:
            self._impl.reap_all()

    @property
    def total_cost_usd(self) -> float | None:
        if self._impl is None:
            return None
        return self._impl.total_cost_usd
