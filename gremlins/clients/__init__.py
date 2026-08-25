from __future__ import annotations

from functools import partial

from gremlins.clients.registry import register_client_factory

# Default tool allowlists (moved from deleted gremlins.permissions.defaults).
# These are the only tools each backend permits; permission enforcement is handled
# by the worktree/cwd containment in the rig client backend.
_DEFAULT_ALLOWED_TOOLS: list[str] = ["Bash", "Edit", "Read", "Write", "Grep", "Glob"]
_DEFAULT_BLOCK: dict[str, list[str]] = {"allowed_tools": _DEFAULT_ALLOWED_TOOLS}


def _make_openai_compatible_client(
    provider: str,
    model: str | None,
    extra_params: dict[str, str] | None = None,
    *,
    default_model: str = "",
) -> object:
    from _gremlins_core.clients import RustClient

    return RustClient(
        provider,
        model or default_model,
        dict(_DEFAULT_BLOCK),
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


register_client_factory("openai", partial(_make_openai_compatible_client, "openai"))
register_client_factory("xai", partial(_make_openai_compatible_client, "xai", default_model="grok-4"))
register_client_factory("openrouter", partial(_make_openai_compatible_client, "openrouter"))
register_client_factory("cmd", _make_cmd_client)
