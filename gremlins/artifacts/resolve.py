"""Resolve interpolation: map entries against the artifact registry."""

from __future__ import annotations

import re

from gremlins.artifacts.registry import ArtifactRegistry, MissingArtifact

_CONTENT_RE = re.compile(r'^content\("([^"]+)"(?:,\s*"([^"]+)")?\)\s*$')


def resolve_interpolation_map(
    artifacts: ArtifactRegistry, interpolation_map: dict[str, str]
) -> dict[str, str]:
    result: dict[str, str] = {}
    for var, raw in interpolation_map.items():
        if raw.startswith("content(") and raw.rstrip().endswith("?"):
            raise ValueError(
                f"Invalid interpolation entry: {raw!r}. "
                f"The '?' default syntax is not supported for content() entries. "
                f"Use a plain URI with '?' and then content() separately on the resolved path."
            )
        m = _CONTENT_RE.match(raw)
        if m:
            uri_str = m.group(1)
            json_path = m.group(2)
            try:
                result[var] = artifacts.content(uri_str, json_path)
            except MissingArtifact:
                raise
            continue

        uri_str, sep, default = raw.partition("?")
        try:
            result[var] = artifacts.read(uri_str)
        except MissingArtifact:
            if not sep:
                raise
            result[var] = default
    return result
