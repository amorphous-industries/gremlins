"""Resolve in: map entries against the artifact registry."""

from __future__ import annotations

import re

from gremlins.artifacts.registry import ArtifactRegistry, MissingArtifact
from gremlins.utils.text import to_str

_READ_SUB = re.compile(r"\{read:([-\w]+)\}")


def _sub_reads(s: str, artifacts: ArtifactRegistry) -> str:
    def _r(m: re.Match[str]) -> str:
        key = m.group(1)
        raw = artifacts.read(key)
        if not isinstance(raw, str):
            raise TypeError(
                f"{{read:{key}}}: expected string artifact, got {type(raw).__name__}"
            )
        return raw.strip()

    return _READ_SUB.sub(_r, s)


def resolve_in_map(
    artifacts: ArtifactRegistry,
    in_map: dict[str, str],
    optional_map: dict[str, str] | None = None,
) -> dict[str, str]:
    result: dict[str, str] = {}
    for var, key in in_map.items():
        resolved_key = _sub_reads(key, artifacts)
        result[var] = to_str(artifacts.read(resolved_key))
    for var, key in (optional_map or {}).items():
        try:
            resolved_key = _sub_reads(key, artifacts)
            result[var] = to_str(artifacts.read(resolved_key))
        except MissingArtifact:
            result[var] = ""
    return result
