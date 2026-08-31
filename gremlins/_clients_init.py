"""Register built-in provider factories into CLIENT_FACTORIES.

Import-time side effect: populates CLIENT_FACTORIES on _gremlins_core.clients
with the four built-in providers (openai, xai, openrouter, cmd).
"""

from __future__ import annotations

from _gremlins_core.clients import CLIENT_FACTORIES, RustClient

_DEFAULT_BLOCK: dict[str, list[str]] = {
    "allowed_tools": ["Bash", "Edit", "Read", "Write", "Grep", "Glob"]
}


def _make_openai(
    provider: str, model: str | None, extra: dict[str, str] | None
) -> RustClient:
    return RustClient(
        provider,
        model or "gpt-4o",
        dict(_DEFAULT_BLOCK),
        extra_params=extra or {},
    )


def _make_cmd(model: str | None, extra: dict[str, str] | None) -> RustClient:
    return RustClient.cmd(model or "")


def _factory_openai(
    model: str | None = None, extra: dict[str, str] | None = None
) -> RustClient:
    return _make_openai("openai", model, extra)


def _factory_xai(
    model: str | None = None, extra: dict[str, str] | None = None
) -> RustClient:
    return _make_openai("xai", model or "grok-4", extra)


def _factory_openrouter(
    model: str | None = None, extra: dict[str, str] | None = None
) -> RustClient:
    return _make_openai("openrouter", model, extra)


def _factory_cmd(
    model: str | None = None, extra: dict[str, str] | None = None
) -> RustClient:
    return _make_cmd(model, extra)


CLIENT_FACTORIES["openai"] = _factory_openai
CLIENT_FACTORIES["xai"] = _factory_xai
CLIENT_FACTORIES["openrouter"] = _factory_openrouter
CLIENT_FACTORIES["cmd"] = _factory_cmd
