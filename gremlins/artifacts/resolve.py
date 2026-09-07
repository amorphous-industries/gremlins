"""Resolve interpolation: map entries against the artifact registry."""

from __future__ import annotations

import json
import pathlib
import re

from gremlins.artifacts.registry import ArtifactRegistry, MissingArtifact

_CONTENT_RE = re.compile(r'^content\("([^"]+)"(?:,\s*"([^"]+)")?\)\s*$')


def _resolve_attr(artifacts: ArtifactRegistry, uri_str: str) -> str:
    """Resolve a URI string that may include a dot-separated attribute path.

    ``ref.name`` reads the artifact at key ``ref``, parses its content as JSON,
    and traverses the path ``name``.
    """
    parts = uri_str.split(".", 1)
    base_key = parts[0]
    path_str = parts[1] if len(parts) > 1 else None

    if path_str is not None:
        first_segment = path_str.split(".", 1)[0]
        if first_segment.startswith("_"):
            raise ValueError(f"private attribute access not allowed: {uri_str!r}")

    raw = artifacts.read(base_key)
    if path_str is None:
        return str(raw)

    # Traverse JSON path
    if isinstance(raw, str):
        try:
            data = json.loads(raw)
        except json.JSONDecodeError:
            # It's a file path, read the file
            data = json.loads(pathlib.Path(raw).read_text(encoding="utf-8"))
    else:
        data = raw

    try:
        for segment in path_str.split("."):
            if isinstance(data, dict):
                data = data[segment]
            else:
                raise ValueError(
                    f"cannot traverse into {type(data)} for path {path_str!r}"
                )
    except (KeyError, IndexError, TypeError) as exc:
        raise MissingArtifact(base_key) from exc
    return str(data) if not isinstance(data, str) else data


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
            result[var] = _resolve_attr(artifacts, uri_str)
        except MissingArtifact:
            if not sep:
                raise
            result[var] = default
    return result
