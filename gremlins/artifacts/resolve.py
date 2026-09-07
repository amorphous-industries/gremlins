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
        # Support optional content() calls: content("uri")? means return "" if missing
        optional = raw.rstrip().endswith("?")
        raw_clean = raw.rstrip().rstrip("?")

        m = _CONTENT_RE.match(raw_clean)
        if m:
            uri_str = m.group(1)
            json_path = m.group(2)
            try:
                result[var] = artifacts.content(uri_str, json_path)
            except MissingArtifact:
                if optional:
                    result[var] = ""
                else:
                    raise
            continue

        key, sep, default = raw.partition("?")
        try:
            value = artifacts.data_uri(key)
            result[var] = value if isinstance(value, str) else str(value)
        except MissingArtifact:
            if not sep:
                raise
            result[var] = default
    return result
