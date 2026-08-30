"""Artifact registry: maps string keys to JSON values, auto-resolving URI strings on read."""

from __future__ import annotations

import json
import logging
import os
import pathlib
import secrets
import shutil
from collections.abc import Iterable, Mapping
from typing import Any

from gremlins.artifacts._protocol import SchemeResolver
from gremlins.artifacts.schemes import (
    FileArtifactResolver,
    GitResolver,
    OpaqueResolver,
)
from gremlins.artifacts.uri import Uri
from gremlins.utils import git as git_utils

logger = logging.getLogger(__name__)


class MissingArtifact(KeyError):
    def __init__(self, key: str) -> None:
        super().__init__(f"artifact not bound: {key!r}")
        self.key = key


class DuplicateArtifact(ValueError):
    def __init__(self, key: str, existing: Any, attempted: Any) -> None:
        super().__init__(
            f"artifact {key!r} already bound to {existing!r}; cannot rebind to {attempted!r}"
        )
        self.key = key


def _extract_scheme(key: str) -> str | None:
    if "://" not in key:
        return None
    return key.split("://", 1)[0]


class ArtifactRegistry:
    def __init__(
        self,
        artifact_dir: pathlib.Path,
        cwd: pathlib.Path | None = None,
        resolvers: Mapping[str, SchemeResolver] | None = None,
    ) -> None:
        self._cwd = cwd
        self.registry_path = artifact_dir.parent / "registry.json"
        self.data: dict[str, Any] = {}
        self._resolvers: dict[str, SchemeResolver] = {
            "file": FileArtifactResolver(artifact_dir),
            "git": GitResolver(cwd),
            **(resolvers or {}),
        }
        if self.registry_path.exists():
            data = json.loads(self.registry_path.read_text(encoding="utf-8"))
            self.data = dict(data)

    def _persist(self) -> None:
        path = self.registry_path
        tmp = path.with_name(path.name + f".{os.getpid()}.{secrets.token_hex(4)}.tmp")
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp.write_text(json.dumps(self.data), encoding="utf-8")
        os.replace(tmp, path)

    def write(self, key: str, value: Any) -> None:
        """Store a JSON value. Fails at write time if value is not JSON-serializable."""
        json.dumps(value)  # validate serializability
        self.data[key] = value
        self._persist()

    def bind(self, key: str, value: Any) -> None:
        """Bind a materialized value to *key*.

        *value* must already be materialized by the appropriate resolver's
        ``materialize()`` — this method stores it directly without further
        transformation.
        """
        if key in self.data:
            if self.data[key] == value:
                return
            raise DuplicateArtifact(key, self.data[key], value)
        self.data[key] = value
        self._persist()

    def mount(self, key: str, value: Any) -> None:
        """Register a binding in-memory only; not persisted to disk."""
        self.data[key] = value

    def read(self, key: str) -> Any:
        if key not in self.data:
            raise MissingArtifact(key)
        value = self.data[key]
        scheme = _extract_scheme(key)
        if scheme is None:
            return value
        if scheme not in self._resolvers:
            self._resolvers[scheme] = OpaqueResolver()
        return self._resolvers[scheme].read(value)

    def produced(self, key: str) -> bool:
        return key in self.data

    def verified(self, key: str) -> bool:
        if key not in self.data:
            return False
        value = self.data[key]
        scheme = _extract_scheme(key)
        if scheme is None:
            return True
        if scheme not in self._resolvers:
            return True
        try:
            self._resolvers[scheme].verify_produced(value)
            return True
        except Exception:
            return False

    def path_for(self, key: str) -> pathlib.Path | None:
        """Return the absolute filesystem path for a file://session/ artifact.

        Returns None if the key is not bound or is not a file:// URI.
        """
        if not key.startswith("file://"):
            return None
        value = self.data.get(key)
        if not isinstance(value, str):
            return None
        return pathlib.Path(value)

    def keys(self) -> Iterable[str]:
        return self.data.keys()

    def resolver(self, scheme: str) -> SchemeResolver:
        if scheme not in self._resolvers:
            self._resolvers[scheme] = OpaqueResolver()
        return self._resolvers[scheme]

    @property
    def file_resolver(self) -> FileArtifactResolver:
        """Return the registry's concrete file resolver."""
        resolver = self._resolvers["file"]
        assert isinstance(resolver, FileArtifactResolver)
        return resolver

    def unbind(self, key: str) -> None:
        if key not in self.data:
            return
        del self.data[key]
        self._persist()

    def bind_git_commit_range(self, key: str, base_sha: str) -> None:
        sha = git_utils.head_sha(cwd=self._cwd)
        if not sha:
            raise RuntimeError("could not resolve HEAD")
        value = f"{base_sha}..{sha}"
        if key in self.data:
            if self.data[key] == value:
                return
            raise DuplicateArtifact(key, self.data[key], value)
        self.data[key] = value
        self._persist()

    # ------------------------------------------------------------------
    # accessor methods
    # ------------------------------------------------------------------

    def raw_entry(self, key: str) -> Any | None:
        """Return the raw stored value for *key*, or None if unbound."""
        return self.data.get(key)

    def get_base_sha(self) -> str:
        value = self.data.get("base_sha")
        return str(value) if value else ""

    def get_base_ref(self) -> str:
        value = self.data.get("base_ref")
        return str(value) if value else ""

    def get_file_contents(self, key: str, *, default: str = "") -> str:
        if not key.startswith("file://"):
            return default
        try:
            return self.read(key)
        except (MissingArtifact, Exception):
            return default

    def merge_from(
        self,
        other: ArtifactRegistry,
        *,
        key_prefix: str = "",
        copy_files: bool = False,
        dest_artifact_dir: pathlib.Path | None = None,
        keys: set[str] | None = None,
    ) -> None:
        """Copy file artifacts and rebind non-file URIs from *other* into self.

        Keys already present in self are skipped. When *key_prefix* is set,
        each incoming key is suffixed with ``"/" + key_prefix``.
        """
        for key in keys if keys is not None else other.keys():
            value = other.raw_entry(key)
            if value is None:
                continue
            bound_key = f"{key}/{key_prefix}" if key_prefix else key
            if bound_key in self.data:
                continue
            if key.startswith("file://session/") and copy_files and isinstance(value, str):
                src = pathlib.Path(value)
                if not src.exists():
                    logger.warning("child artifact missing: %s", src)
                    continue
                dest_dir = dest_artifact_dir or self.file_resolver._artifact_dir
                name = key[len("file://session/"):]
                dest_name = f"{key_prefix}/{name}" if key_prefix else name
                dest = dest_dir / dest_name
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(src, dest)
                self.bind(bound_key, str(dest))
            else:
                try:
                    self.bind(bound_key, value)
                except Exception:
                    logger.warning(
                        "failed to bind %s -> %s into parent registry",
                        bound_key,
                        value,
                        exc_info=True,
                    )

    @classmethod
    def from_registry_file(
        cls,
        path: pathlib.Path,
        *,
        artifact_dir: pathlib.Path,
        cwd: pathlib.Path | None = None,
    ) -> ArtifactRegistry:
        """Load a registry from a registry.json at *path*."""
        registry = cls(artifact_dir=artifact_dir, cwd=cwd)
        if path != registry.registry_path:
            if path.exists():
                registry.data = dict(json.loads(path.read_text(encoding="utf-8")))
                registry._persist()
        return registry
