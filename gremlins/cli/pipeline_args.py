from __future__ import annotations

import logging
import pathlib

from _gremlins_core.discovery import resolve_pipeline_path
from _gremlins_core.schemas import Pipeline

logger = logging.getLogger(__name__)


def resolve_pipeline(
    kind: str, pipeline_args: tuple[str, ...], project_root: str
) -> tuple[list[str], str]:
    args = list(pipeline_args)
    pipeline_val: str | None = None
    filtered: list[str] = []
    i = 0
    while i < len(args):
        if args[i] == "--pipeline":
            if i + 1 < len(args):
                pipeline_val = args[i + 1]
                i += 2
            else:
                i += 1
        elif args[i].startswith("--pipeline="):
            pipeline_val = args[i][len("--pipeline=") :]
            i += 1
        else:
            filtered.append(args[i])
            i += 1
    name = pipeline_val or kind
    resolved = str(resolve_pipeline_path(name, pathlib.Path(project_root)))
    return filtered, resolved


def extract_arg_value(args: list[str], flag: str) -> str:
    value = ""
    i = 0
    while i < len(args):
        arg = args[i]
        if arg == flag:
            if i + 1 < len(args):
                value = args[i + 1]
                i += 2
                continue
            i += 1
            continue
        prefix = f"{flag}="
        if arg.startswith(prefix):
            value = arg[len(prefix) :]
        i += 1
    return value


def extract_client_spec(args: list[str]) -> str:
    return extract_arg_value(args, "--client")


def load_prefix_clients() -> tuple[dict[str, str], dict[str, str]]:
    """Extract client rules from the global config.

    Returns ``(exact_map, prefix_map)`` — exact stage-name keys and
    prefix globs (with the ``*`` stripped).  Returns two empty dicts
    when no config file exists or no rules are defined.  Delegates to
    ``_gremlins_core.config.get_config().default_client_by_stage()``,
    which handles validation.
    """
    from _gremlins_core.config import get_config

    return get_config().default_client_by_stage()


def launch_client_label(pipeline_args: list[str], pipeline: Pipeline | None) -> str:
    from _gremlins_core.config import get_config, user_config_root

    client_spec = extract_client_spec(pipeline_args)
    if client_spec:
        return client_spec
    global_client = get_config().default_client
    if global_client:
        return global_client
    if pipeline and pipeline.default_client:
        return str(pipeline.default_client)

    config_path = pathlib.Path(user_config_root()) / "config.json"
    raise ValueError(
        "no client configured — pass --client, set default-client in "
        f"{config_path}, or ensure the pipeline declares "
        "default-client"
    )
