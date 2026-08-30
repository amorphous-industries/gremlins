"""Bootstrap block: CLI contract plus launch-only and every-worktree setup."""

from __future__ import annotations

import dataclasses
import os
import pathlib
from collections.abc import Mapping
from typing import Any, cast

_BOOTSTRAP_KEYS = frozenset({"source", "launch_cmds", "cmds", "cli_out"})
_MAPPING_ERROR = (
    "'bootstrap' must be a mapping with optional source:/launch_cmds:/cmds:/cli_out:"
)
_VALID_SOURCE_TYPES = frozenset({"filepath", "string"})


@dataclasses.dataclass
class InputSource:
    name: str
    types: list[str]
    optional: bool = False

    def __post_init__(self) -> None:
        if not self.types:
            raise ValueError(
                f"input source {self.name!r}: types list must not be empty"
            )
        for t in self.types:
            if t not in _VALID_SOURCE_TYPES:
                raise ValueError(
                    f"input source {self.name!r}: unknown type {t!r}. "
                    f"Supported types: {', '.join(sorted(_VALID_SOURCE_TYPES))}"
                )


class InputSources:
    def __init__(self, sources: dict[str, InputSource] | None = None) -> None:
        self.sources = sources or {}

    @classmethod
    def from_yaml(cls, raw: dict[str, Any]) -> InputSources:
        sources: dict[str, InputSource] = {}
        for key, entry in raw.items():
            if not isinstance(entry, dict):
                raise ValueError(
                    f"input source {key!r}: expected a mapping, got {type(entry).__name__}"
                )
            entry = cast(dict[str, Any], entry)

            type_raw = entry.get("type")
            if type_raw is None:
                raise ValueError(f"input source {key!r}: missing required 'type' field")

            if isinstance(type_raw, str):
                types = [type_raw]
            elif isinstance(type_raw, list):
                type_raw = cast(list[Any], type_raw)
                if not type_raw:
                    raise ValueError(
                        f"input source {key!r}: type list must not be empty"
                    )
                for t in type_raw:
                    if not isinstance(t, str):
                        raise ValueError(
                            f"input source {key!r}: all type entries must be strings"
                        )
                types = cast(list[str], type_raw)
            else:
                raise ValueError(
                    f"input source {key!r}: 'type' must be a string or list of strings, "
                    f"got {type(type_raw).__name__}"
                )

            optional_raw = entry.get("optional", False)
            if not isinstance(optional_raw, bool):
                raise ValueError(
                    f"input source {key!r}: 'optional' must be a boolean, "
                    f"got {type(optional_raw).__name__}"
                )
            sources[key] = InputSource(name=key, types=types, optional=optional_raw)

        return cls(sources)

    def get(self, key: str) -> InputSource | None:
        return self.sources.get(key)

    def all_sources(self) -> list[str]:
        return list(self.sources.keys())

    def required_sources(self) -> list[str]:
        return [name for name, src in self.sources.items() if not src.optional]


@dataclasses.dataclass
class Bootstrap:
    source: InputSources | None = None
    launch_cmds: list[str] = dataclasses.field(default_factory=list[str])
    cmds: list[str] = dataclasses.field(default_factory=list[str])
    cli_out: dict[str, str] = dataclasses.field(default_factory=dict[str, str])
    cli_out_optional: dict[str, str] = dataclasses.field(default_factory=dict[str, str])

    @classmethod
    def from_yaml(cls, raw: object) -> Bootstrap:
        if raw is None:
            return cls()
        if not isinstance(raw, dict):
            raise ValueError(_MAPPING_ERROR)
        raw = cast(dict[str, Any], raw)
        if "out" in raw:
            raise ValueError("'bootstrap.out' is not valid; use 'cli_out'")
        unknown = set(raw) - _BOOTSTRAP_KEYS
        if unknown:
            keys = ", ".join(sorted(repr(k) for k in unknown))
            raise ValueError(f"unknown bootstrap key(s): {keys}")

        source: InputSources | None = None
        source_raw = raw.get("source")
        if source_raw is not None:
            if not isinstance(source_raw, dict):
                raise ValueError("'bootstrap.source' must be a mapping")
            source = InputSources.from_yaml(cast(dict[str, Any], source_raw))

        cli_out_raw = raw.get("cli_out")
        if cli_out_raw is None:
            cli_out: dict[str, str] = {}
            cli_out_optional: dict[str, str] = {}
        elif not isinstance(cli_out_raw, dict):
            raise ValueError("'bootstrap.cli_out' must be a mapping")
        else:
            cli_out_dict = cast(dict[str, Any], cli_out_raw)
            cli_out_optional_raw = cast(
                dict[str, Any], cli_out_dict.get("optional", {})
            )
            cli_out_optional = {str(k): str(v) for k, v in cli_out_optional_raw.items()}
            cli_out = {
                str(k): str(v) for k, v in cli_out_dict.items() if k != "optional"
            }

        return cls(
            source=source,
            launch_cmds=_string_list(raw.get("launch_cmds"), "bootstrap.launch_cmds"),
            cmds=_string_list(raw.get("cmds"), "bootstrap.cmds"),
            cli_out=cli_out,
            cli_out_optional=cli_out_optional,
        )


def _string_list(raw: object, label: str) -> list[str]:
    if raw is None:
        return []
    if not isinstance(raw, list):
        raise ValueError(f"{label!r} must be a list of strings")
    cmds: list[str] = []
    for item in cast(list[object], raw):
        if not isinstance(item, str):
            raise ValueError(f"{label!r} must be a list of strings")
        cmds.append(item)
    return cmds


def source_env(
    source: InputSources | None, values: Mapping[str, Any]
) -> dict[str, str]:
    """Env vars for launch_cmds: source key → value. Omitted optionals are absent."""
    if source is None:
        return {}
    env: dict[str, str] = {}
    for key in source.sources:
        value = values.get(key)
        if value is None or value == "":
            continue
        env[key] = str(value)
    return env


def validate_source_values(
    source: InputSources | None, values: Mapping[str, Any]
) -> None:
    """Validate CLI/source values. filepath-only requires an existing file."""
    if source is None:
        return
    for key, src in source.sources.items():
        value = values.get(key)
        if value is None or value == "":
            if not src.optional:
                raise ValueError(f"required bootstrap.source {key!r} is not available")
            continue
        if src.types == ["filepath"] and not os.path.isfile(str(value)):
            raise ValueError(
                f"bootstrap.source {key!r}: expected an existing file, got {value!r}"
            )


def substitute_bootstrap_vars(
    cmd: str, *, artifact_dir: pathlib.Path, cwd: pathlib.Path
) -> str:
    return cmd.replace("{artifact_dir}", str(artifact_dir)).replace("{cwd}", str(cwd))
