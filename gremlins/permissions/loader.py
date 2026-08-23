from __future__ import annotations

import pathlib
from typing import Any

from gremlins import paths
from gremlins.permissions.policy import Policy
from gremlins.utils.yaml_io import load_yaml_file

_DEFAULTS_DIR = pathlib.Path(__file__).parent / "defaults"


def load_policy(
    *,
    cli_permissions_file: pathlib.Path | None,
    cwd: pathlib.Path,
) -> Policy:
    blocks = (
        _blocks_from_file(cli_permissions_file)
        if cli_permissions_file is not None
        else _blocks_from_project(cwd)
    )
    return Policy(blocks=blocks)


def has_default_block(provider: str) -> bool:
    return (_DEFAULTS_DIR / f"{provider}.yaml").exists()


def load_default_block(provider: str) -> dict[str, Any]:
    return load_yaml_file(_DEFAULTS_DIR / f"{provider}.yaml")

def _blocks_from_project(cwd: pathlib.Path) -> dict[str, dict[str, Any]]:
    project_file = paths.project_overlay_dir(cwd) / "permissions.yaml"
    if not project_file.exists():
        return {}
    data = load_yaml_file(project_file)
    return dict(data.get("blocks", {}))


def _blocks_from_file(path: pathlib.Path | None) -> dict[str, dict[str, Any]]:
    if path is None or not path.exists():
        return {}
    data = load_yaml_file(path)
    return dict(data.get("blocks", {}))

