from __future__ import annotations

from gremlins.clients.registry import register_client_factory
from gremlins.permissions.policy import Policy


def _openai_instructions() -> str:
    from gremlins.utils.yaml_io import load_bundled_prompt

    return load_bundled_prompt("default_openai_agents_instructions.md")


def _make_openai_client(model: str | None, policy: Policy) -> object:
    from _gremlins_core.clients import RustClient

    from gremlins.permissions.loader import load_default_block

    return RustClient(
        "openai",
        model or "",
        load_default_block("openai") | policy.block_for("openai"),
        instructions=_openai_instructions(),
    )


def _make_xai_client(model: str | None, policy: Policy) -> object:
    from _gremlins_core.clients import RustClient

    from gremlins.permissions.loader import load_default_block

    return RustClient(
        "xai",
        model or "grok-4",
        load_default_block("xai") | policy.block_for("xai"),
        instructions=_openai_instructions(),
    )


def _make_openrouter_client(model: str | None, policy: Policy) -> object:
    from _gremlins_core.clients import RustClient

    from gremlins.permissions.loader import load_default_block

    return RustClient(
        "openrouter",
        model or "",
        load_default_block("openrouter") | policy.block_for("openrouter"),
        instructions=_openai_instructions(),
    )


def _make_cmd_client(command: str | None, policy: Policy) -> object:
    # cmd: bypass/flags are spelled in the command template; policy is unused.
    if not command:
        raise ValueError("cmd: command is required")
    from _gremlins_core.clients import RustClient

    return RustClient.cmd(command)


register_client_factory("openai", _make_openai_client)
register_client_factory("xai", _make_xai_client)
register_client_factory("openrouter", _make_openrouter_client)
register_client_factory("cmd", _make_cmd_client)
