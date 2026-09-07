"""Artifact registry: maps URI strings to slugged filesystem paths."""

from __future__ import annotations

import json
import logging
import os
import pathlib
import secrets
import shutil
from collections.abc import Iterable
from typing import Any

from _gremlins_core.artifacts import Uri

from gremlins.artifacts.schemes import FileArtifactResolver

logger = logging.getLogger(__name__)


class MissingArtifact(KeyError):
    def __init__(self, key: str) -> None:
        super().__init__(f"artifact not bound: {key!r}")
        self.key = key


class ArtifactRegistry:
    def __init__(
        self,
        artifact_dir: pathlib.Path,
    ) -> None:
        self.registry_path = artifact_dir.parent / "registry.json"
        self.data: dict[str, Any] = {}
        self._file_resolver = FileArtifactResolver(artifact_dir)
        if self.registry_path.exists():
            data = json.loads(self.registry_path.read_text(encoding="utf-8"))
            self.data = dict(data)

    def _persist(self) -> None:
        path = self.registry_path
        tmp = path.with_name(path.name + f".{os.getpid()}.{secrets.token_hex(4)}.tmp")
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp.write_text(json.dumps(self.data), encoding="utf-8")
        os.replace(tmp, path)

    def register(self, uri: Uri) -> str:
        """Register a URI artifact, returning the slugged filesystem path.

        Every call generates a fresh slug so that each loop re-entry
        (or any repeated binding of the same logical URI) writes to a
        distinct on-disk filename.
        """
        key = str(uri)
        base = self._file_resolver.path_for(uri)
        slug = secrets.token_hex(4)
        slugged = base.parent / f"{slug}_{base.name}"
        self.data[key] = str(slugged)
        self._persist()
        return self.data[key]

    def read(self, uri_str: str) -> str:
        """Return the resolved value (filesystem path for file URIs)."""
        if uri_str not in self.data:
            raise MissingArtifact(uri_str)
        return self.data[uri_str]

    def content(self, uri_str: str, json_path: str | None = None) -> str:
        """Read file content, optionally traversing a JSON path."""
        path = self.read(uri_str)
        raw = pathlib.Path(path).read_text(encoding="utf-8")
        if json_path is None:
            return raw
        data = json.loads(raw)
        for segment in json_path.split("."):
            if isinstance(data, dict):
                data = data[segment]
            else:
                raise ValueError(
                    f"content({uri_str}, {json_path!r}): cannot traverse into {type(data)}"
                )
        return str(data) if not isinstance(data, str) else data

    def exists(self, uri: str | Uri) -> bool:
        """Check whether a registered artifact's on-disk file exists and is non-empty."""
        match uri:
            case str():
                key = uri
            case Uri():
                key = str(uri)
            case _:
                raise ValueError(f"expected str or Uri, got {type(uri).__name__}")
        stored = self.data.get(key)
        if stored is None or not isinstance(stored, str):
            return False
        p = pathlib.Path(stored)
        return p.exists() and p.stat().st_size > 0

    def produced(self, key: str) -> bool:
        return key in self.data

    def verified(self, key: str) -> bool:
        if key not in self.data:
            return False
        value = self.data[key]
        if not isinstance(value, str):
            return True
        p = pathlib.Path(value)
        return p.exists() and p.stat().st_size > 0

    def keys(self) -> Iterable[str]:
        return self.data.keys()

    def unbind(self, key: str) -> None:
        if key not in self.data:
            return
        del self.data[key]
        self._persist()

    def raw_entry(self, key: str) -> Any | None:
        return self.data.get(key)

    @property
    def file_resolver(self) -> FileArtifactResolver:
        return self._file_resolver

    def merge_from(
        self,
        other: ArtifactRegistry,
        *,
        key_map: dict[str, str] | None = None,
        copy_files: bool = False,
        keys: set[str] | None = None,
    ) -> None:
        """Copy file artifacts from *other* into self.

        *key_map* maps child keys to parent keys.  When None, child keys are
        used as-is (identity mapping).  Keys already present in self are skipped.
        """
        for key in keys if keys is not None else other.keys():
            uri_str = other.raw_entry(key)
            if not isinstance(uri_str, str):
                continue
            parent_key = key_map[key] if key_map else key
            if parent_key in self.data:
                continue
            if copy_files:
                src_path = pathlib.Path(uri_str)
                if not src_path.exists():
                    logger.warning("child artifact missing: %s", src_path)
                    continue
                dest_path = pathlib.Path(self.register(Uri.parse(parent_key)))
                dest_path.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(src_path, dest_path)
            else:
                self.data[parent_key] = uri_str
                self._persist()

    @classmethod
    def from_registry_file(
        cls,
        path: pathlib.Path,
        *,
        artifact_dir: pathlib.Path,
    ) -> ArtifactRegistry:
        """Load a registry from a registry.json at *path*."""
        registry = cls(artifact_dir=artifact_dir)
        if path != registry.registry_path:
            if path.exists():
                registry.data = dict(json.loads(path.read_text(encoding="utf-8")))
                registry._persist()
        return registry
