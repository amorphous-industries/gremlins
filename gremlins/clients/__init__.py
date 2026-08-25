from __future__ import annotations

from gremlins.clients.registry import register_client_factory

# Default tool allowlists (moved from deleted gremlins.permissions.defaults).
# These are the only tools each backend permits; permission enforcement is handled
# by the worktree/cwd containment in the rig client backend.
_DEFAULT_ALLOWED_TOOLS: list[str] = ["Bash", "Edit", "Read", "Write", "Grep", "Glob"]
_DEFAULT_BLOCK: dict[str, list[str]] = {"allowed_tools": _DEFAULT_ALLOWED_TOOLS}


def _openai_instructions() -> str:
    from gremlins.utils.yaml_io import load_bundled_prompt

    return load_bundled_prompt("default_openai_agents_instructions.md")


def _make_openai_client(
    model: str | None, extra_params: dict[str, str] | None = None
) -> object:
    from _gremlins_core.clients import RustClient

    return RustClient(
        "openai",
        model or "",
        dict(_DEFAULT_BLOCK),
        instructions=_openai_instructions(),
        extra_params=extra_params or {},
    )


def _make_xai_client(
    model: str | None, extra_params: dict[str, str] | None = None
) -> object:
    from _gremlins_core.clients import RustClient

    return RustClient(
        "xai",
        model or "grok-4",
        dict(_DEFAULT_BLOCK),
        instructions=_openai_instructions(),
        extra_params=extra_params or {},
    )


def _make_openrouter_client(
    model: str | None, extra_params: dict[str, str] | None = None
) -> object:
    from _gremlins_core.clients import RustClient

    return RustClient(
        "openrouter",
        model or "",
        dict(_DEFAULT_BLOCK),
        instructions=_openai_instructions(),
        extra_params=extra_params or {},
    )


def _make_cmd_client(
    model: str | None, extra_params: dict[str, str] | None = None
) -> object:
    # "model" holds the full shell command for the cmd provider.
    # extra_params is accepted for signature uniformity but ignored — cmd
    # is exempt from param parsing in Client.parse().
    if not model:
        raise ValueError("cmd: command is required")
    from _gremlins_core.clients import RustClient

    return RustClient.cmd(model)


register_client_factory("openai", _make_openai_client)
register_client_factory("xai", _make_xai_client)
register_client_factory("openrouter", _make_openrouter_client)
register_client_factory("cmd", _make_cmd_client)
