"""Artifact registry: maps string keys to JSON values, auto-resolving URI strings on read."""

from __future__ import annotations

import json
import logging
import os
import pathlib
import secrets
import shutil
from collections.abc import Iterable, Mapping
from typing import Any, cast

from gremlins.artifacts._protocol import SchemeResolver
from gremlins.artifacts.schemes import (
    FileArtifactResolver,
    GhOpaqueResolver,
    GitResolver,
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
            "gh": GhOpaqueResolver(),
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

    def bind(self, key: str, uri: Uri) -> None:
        value = str(uri)
        if key in self.data:
            if self.data[key] == value:
                return
            raise DuplicateArtifact(key, self.data[key], value)
        self.data[key] = value
        self._persist()

    def mount(self, key: str, uri: Uri) -> None:
        """Register a URI binding in-memory only; not persisted to disk."""
        self.data[key] = str(uri)

    def resolve(self, key: str) -> Uri:
        if key not in self.data:
            raise MissingArtifact(key)
        value = self.data[key]
        if not isinstance(value, str):
            raise ValueError(f"artifact {key!r} is not a URI (stored value: {value!r})")
        return Uri.parse(value)

    def _resolve_value(self, value: Any) -> Any:
        if not isinstance(value, str):
            return value
        try:
            uri = Uri.parse(value)
        except ValueError:
            return value
        if uri.scheme not in self._resolvers:
            return value
        resolved = self._resolvers[uri.scheme].read(uri)
        return self._resolve_value(resolved)

    def read(self, key: str) -> Any:
        if key not in self.data:
            raise MissingArtifact(key)
        return self._resolve_value(self.data[key])

    def produced(self, key: str) -> bool:
        return key in self.data

    def verified(self, key: str) -> bool:
        if key not in self.data:
            return False
        value = self.data[key]
        if not isinstance(value, str):
            return True
        try:
            uri = Uri.parse(value)
        except ValueError:
            return True
        if uri.scheme not in self._resolvers:
            return True
        try:
            self._resolvers[uri.scheme].verify_produced(uri)
            return True
        except Exception:
            return False

    def path_for(self, key: str) -> pathlib.Path | None:
        """Return the absolute filesystem path for a file://session/ artifact.

        Returns None if the key is not bound or does not resolve to a
        file://session/ URI.
        """
        try:
            uri = self.resolve(key)
        except MissingArtifact:
            return None
        if uri.scheme != "file" or not uri.path.startswith("session/"):
            return None
        return self.file_resolver.path_for(uri)

    def keys(self) -> Iterable[str]:
        return self.data.keys()

    def resolver(self, scheme: str) -> SchemeResolver:
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
        self.bind(key, Uri.parse(f"git://range/{base_sha}..{sha}"))

    # ------------------------------------------------------------------
    # accessor methods
    # ------------------------------------------------------------------

    def raw_entry(self, key: str) -> Any | None:
        """Return the raw stored value for *key*, or None if unbound."""
        return self.data.get(key)

    def get_base_sha(self) -> str:
        uri_str = self.data.get("base_sha")
        if not uri_str or not isinstance(uri_str, str):
            return ""
        if uri_str.startswith("git://commit/"):
            return uri_str.removeprefix("git://commit/")
        return ""

    def get_base_ref(self) -> str:
        uri_str = self.data.get("base_ref")
        if not uri_str or not isinstance(uri_str, str):
            return ""
        if uri_str.startswith("git://ref/"):
            return uri_str.removeprefix("git://ref/")
        return ""

    def get_pr_url(self) -> str | None:
        for key in ("pr-url", "pr"):
            try:
                value = self.read(key)
            except MissingArtifact:
                continue
            if isinstance(value, str):
                return value
            if isinstance(value, dict):
                val = cast(dict[str, Any], value)
                uri = val.get("uri")
                if isinstance(uri, str):
                    return uri
                url = val.get("url")
                if isinstance(url, str):
                    return url
        return None

    def get_file_contents(self, key: str, *, default: str = "") -> str:
        try:
            uri = self.resolve(key)
        except MissingArtifact:
            return default
        if uri.scheme != "file":
            return default
        try:
            return self._resolvers["file"].read(uri)
        except Exception:
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
            uri_str = other.raw_entry(key)
            if not isinstance(uri_str, str):
                continue
            bound_key = f"{key}/{key_prefix}" if key_prefix else key
            if bound_key in self.data:
                continue
            if uri_str.startswith("file://session/") and copy_files:
                name = uri_str[len("file://session/") :]
                src = other.file_resolver.path_for(Uri.parse(uri_str))
                if not src.exists():
                    logger.warning("child artifact missing: %s", src)
                    continue
                dest_dir = dest_artifact_dir or self.file_resolver.path_for(
                    Uri.parse("file://session/")
                )
                dest_name = f"{key_prefix}/{name}" if key_prefix else name
                dest = dest_dir / dest_name
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(src, dest)
                self.bind(bound_key, Uri.parse(f"file://session/{dest_name}"))
            else:
                try:
                    self.bind(bound_key, Uri.parse(uri_str))
                except Exception:
                    logger.warning(
                        "failed to bind %s -> %s into parent registry",
                        bound_key,
                        uri_str,
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
