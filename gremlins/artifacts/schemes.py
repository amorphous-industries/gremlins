"""Concrete SchemeResolver implementation for artifact:// URIs."""

from __future__ import annotations

import pathlib

from _gremlins_core.artifacts import Uri


class FileArtifactResolver:
    """Resolves artifact:// URIs against a fixed artifact directory."""

    def __init__(self, artifact_dir: pathlib.Path) -> None:
        self._artifact_dir = artifact_dir

    def _path(self, uri: Uri) -> pathlib.Path:
        name = uri.path.lstrip("/")
        p = (self._artifact_dir / name).resolve()
        base = self._artifact_dir.resolve()
        try:
            p.relative_to(base)
        except ValueError:
            raise ValueError(f"path escapes artifact directory: {uri}") from None
        return p

    def path_for(self, uri: Uri) -> pathlib.Path:
        """Return the on-disk path an artifact:// URI resolves to."""
        return self._path(uri)
