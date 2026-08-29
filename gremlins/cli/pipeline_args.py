from __future__ import annotations

import json
import pathlib
from typing import Any, cast

from gremlins.paths import user_config_root
from gremlins.pipeline import Pipeline
from gremlins.pipeline.discovery import resolve_pipeline_path


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


def _load_global_config() -> dict[str, Any]:
    path = user_config_root() / "config.json"
    try:
        with open(path) as f:
            data = json.load(f)
    except FileNotFoundError:
        return {}
    if not isinstance(data, dict):
        raise ValueError(
            f"config file {path} must contain a JSON object, got {type(data).__name__}"
        )
    return cast(dict[str, Any], data)


def launch_client_label(pipeline_args: list[str], pipeline: Pipeline | None) -> str:
    client_spec = extract_client_spec(pipeline_args)
    if client_spec:
        return client_spec
    cfg = _load_global_config()
    global_client = cfg.get("default_client", "") or ""
    if global_client:
        return str(global_client)
    if pipeline and pipeline.default_client:
        return str(pipeline.default_client)
    raise ValueError(
        "no client configured — pass --client or ensure the pipeline declares default_client"
    )
