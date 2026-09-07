"""Concrete SchemeResolver implementation for artifact:// URIs."""

from __future__ import annotations

import pathlib

from _gremlins_core.artifacts import Uri


class FileArtifactResolver:
    """Resolves artifact:// URIs against a fixed artifact directory."""

    def __init__(self, artifact_dir: pathlib.Path) -> None:
        self._artifact_dir = artifact_dir

    def _path(self, uri: Uri) -> pathlib.Path:
        if uri.path.startswith("/"):
            return pathlib.Path(uri.path).resolve()
        name = uri.path.lstrip("/")
        if name.startswith("session/"):
            name = name[len("session/"):]
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

    def read(self, uri: Uri) -> str:
        """Read the file content for a URI."""
        p = self._path(uri)
        return p.read_text(encoding="utf-8")

    def verify_produced(self, uri: Uri) -> None:
        """Verify a file artifact exists and has content.

        Raises FileNotFoundError if the path does not exist or is empty.
        """
        p = self._path(uri)
        if not p.exists() or p.stat().st_size == 0:
            raise FileNotFoundError(f"artifact not produced: {uri}")
