"""Artifact registry: maps string keys to JSON values, auto-resolving URI strings on read."""

from __future__ import annotations

import json
import logging
import os
import pathlib
import secrets
from collections.abc import Iterable, Mapping
from typing import Any

from gremlins.artifacts._protocol import SchemeResolver
from gremlins.artifacts.schemes import (
    FileArtifactResolver,
    GitResolver,
    OpaqueResolver,
)
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

        Resolves short keys through the registry first. Returns None if the
        key is not bound or does not resolve to a file path.
        """
        value = self.data.get(key)
        if value is None:
            return None
        if isinstance(value, str) and value.startswith("file://"):
            return pathlib.Path(value)
        # Short key — the stored value might be a materialized file path.
        if isinstance(value, str):
            p = pathlib.Path(value)
            if p.exists():
                return p
        return None

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
        self.bind(key, f"{base_sha}..{sha}")

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
