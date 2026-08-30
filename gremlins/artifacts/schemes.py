"""Concrete SchemeResolver implementations for file://, git://, and opaque URIs."""

from __future__ import annotations

import pathlib
from typing import Any

from gremlins.artifacts.uri import Uri
from gremlins.utils import git as git_utils
from gremlins.utils import proc


class FileArtifactResolver:
    """Resolves file://session/<name> against a fixed artifact directory."""

    def __init__(self, artifact_dir: pathlib.Path) -> None:
        self._artifact_dir = artifact_dir

    def _path_from_uri_str(self, uri_str: str) -> pathlib.Path:
        uri = Uri.parse(uri_str)
        return self._path(uri)

    def _path(self, uri: Uri) -> pathlib.Path:
        if uri.path.startswith("/"):
            return pathlib.Path(uri.path).resolve()
        if not uri.path.startswith("session/"):
            raise ValueError(f"file:// URI must start with 'session/': {uri}")
        name = uri.path[len("session/") :]
        p = (self._artifact_dir / name).resolve()
        base = self._artifact_dir.resolve()
        try:
            p.relative_to(base)
        except ValueError:
            raise ValueError(f"path escapes artifact directory: {uri}") from None
        return p

    def path_for(self, uri: Uri) -> pathlib.Path:
        """Return the on-disk path a file:// URI resolves to."""
        return self._path(uri)

    def materialize(self, uri_str: str) -> str:
        """Compute the real filesystem path once, at bind time."""
        return str(self._path_from_uri_str(uri_str))

    def read(self, value: Any) -> str:
        if not isinstance(value, str):
            return str(value)
        try:
            return pathlib.Path(value).read_text(encoding="utf-8")
        except FileNotFoundError:
            return ""

    def write(self, uri: Uri, content: str) -> None:
        p = self._path(uri)
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(content, encoding="utf-8")

    def verify_produced(self, value: Any) -> None:
        if not isinstance(value, str):
            return
        p = pathlib.Path(value)
        if not p.exists() or p.stat().st_size == 0:
            raise FileNotFoundError(f"artifact file missing or empty: {p}")


class GitResolver:
    """Resolves git://range/<base>..<head>, git://ref/<name>, git://commit/<sha>."""

    def __init__(self, cwd: pathlib.Path | None = None) -> None:
        self._cwd = cwd

    def materialize(self, uri_str: str) -> str:
        uri = Uri.parse(uri_str)
        path = uri.path
        if path.startswith("ref/"):
            name = path.removeprefix("ref/")
            proc.run_or_raise(["git", "rev-parse", name], cwd=self._cwd)
            return name
        if path.startswith("commit/"):
            return path.removeprefix("commit/")
        if path.startswith("range/"):
            return path.removeprefix("range/")
        raise ValueError(f"unrecognised git URI path: {uri}")

    def read(self, value: Any) -> Any:
        if not isinstance(value, str):
            return value
        if ".." in value:
            # git://range/<base>..<head> — materialized range string
            out = proc.run_or_raise(
                ["git", "log", "--format=%H %s", value], cwd=self._cwd
            )
            commits: list[dict[str, str]] = []
            for line in out.splitlines():
                sha, _, subject = line.partition(" ")
                commits.append({"sha": sha, "subject": subject})
            return commits
        # ref name or commit sha — stored as bare string
        return value

    def verify_produced(self, value: Any) -> None:
        self.read(value)


class OpaqueResolver:
    """Resolves opaque URIs: returns the bare URI string.

    Used for built-in URI schemes that have no concrete resolver
    (currently ``gh://``)."""

    def materialize(self, uri_str: str) -> str:
        return uri_str

    def read(self, value: Any) -> str:
        return str(value) if value is not None else ""

    def verify_produced(self, value: Any) -> None:
        pass


def snapshot_head_before(cwd: pathlib.Path | None = None) -> str:
    """Return current HEAD sha for use with ArtifactRegistry.bind_git_commit_range()."""
    sha = git_utils.head_sha(cwd=cwd)
    if not sha:
        raise RuntimeError("could not resolve HEAD")
    return sha
