from __future__ import annotations

import os
import pathlib
import re
from typing import TYPE_CHECKING, Any, cast

from gremlins.artifacts.registry import (
    ArtifactRegistry,
    DuplicateArtifact,
    MissingArtifact,
)
from gremlins.artifacts.resolve import resolve_in_map
from gremlins.artifacts.schemes import snapshot_head_before
from gremlins.artifacts.uri import Uri
from gremlins.stages.base import Stage
from gremlins.stages.constants import FRAMEWORK_KEYS
from gremlins.stages.outcome import Bail, Done, Outcome
from gremlins.utils import proc as _proc

if TYPE_CHECKING:
    from gremlins.executor.gremlin import Gremlin

_READ_SUB = re.compile(r"\{read:([-\w]+)\}")
_ARTIFACT_SUB = re.compile(r"\{artifact:([-\w]+)\}")
_BAIL_KEY = "bail"


def _sub_reads(s: str, artifacts: ArtifactRegistry) -> str:
    def _r(m: re.Match[str]) -> str:
        key = m.group(1)
        raw = artifacts.read(key)
        if not isinstance(raw, str):
            raise TypeError(
                f"{{read:{key}}}: expected string artifact, got {type(raw).__name__}"
            )
        return raw.strip()

    return _READ_SUB.sub(_r, s)


def _sub_artifact_paths(s: str, artifacts: ArtifactRegistry) -> str:
    """Replace {artifact:key} with the absolute filesystem path of a
    file://session/ artifact.

    Raises MissingArtifact when the key is not bound.
    Raises ValueError when the key is bound but does not resolve to a
    file://session/ URI (e.g. gh:// or git:// artifacts).
    """

    def _r(m: re.Match[str]) -> str:
        key = m.group(1)
        p = artifacts.path_for(key)
        if p is None:
            raise ValueError(
                f"{{artifact:{key}}}: artifact is not a file://session/ path"
            )
        return str(p)

    return _ARTIFACT_SUB.sub(_r, s)


class Exec(Stage):
    type = "exec"

    def __init__(
        self,
        name: str,
        options: dict[str, Any],
        *,
        in_map: dict[str, str] | None = None,
        in_optional_map: dict[str, str] | None = None,
        out_map: dict[str, str] | None = None,
        out_optional_map: dict[str, str] | None = None,
    ) -> None:
        super().__init__(name)
        self.options = options
        self.in_map = in_map or {}
        self.in_optional_map = in_optional_map or {}
        self.out_map = out_map or {}
        self.out_optional_map = out_optional_map or {}

    @classmethod
    def with_dict(cls, d: dict[str, Any], depth: int = 0) -> Exec:
        name = d.get("name") or ""
        raw_in: object = d.get("in") or {}
        raw_out: object = d.get("out") or {}
        if not isinstance(raw_in, dict):
            raise ValueError(f"stage {name!r}: 'in' must be a mapping")
        if not isinstance(raw_out, (dict, list)):
            raise ValueError(f"stage {name!r}: 'out' must be a mapping or list")
        for k in cast(dict[str, Any], d.get("options") or {}):
            if k in FRAMEWORK_KEYS:
                raise ValueError(
                    f"stage {name!r}: option key {k!r} collides with framework substitution variable"
                )
        in_dict = cast(dict[str, str], raw_in)
        in_optional = {
            str(k): str(v)
            for k, v in cast(dict[str, Any], in_dict.pop("optional", {})).items()
        }
        if isinstance(raw_out, list):
            out_list = [str(v) for v in cast(list[Any], raw_out)]
            out_map = {v: v for v in out_list}
            out_optional: dict[str, str] = {}
        else:
            out_dict = cast(dict[str, str], raw_out)
            out_optional = {
                str(k): str(v)
                for k, v in cast(dict[str, Any], out_dict.pop("optional", {})).items()
            }
            out_map = {str(k): str(v) for k, v in out_dict.items()}
        return cls(
            name,
            d.get("options") or {},
            in_map=in_dict,
            in_optional_map=in_optional,
            out_map=out_map,
            out_optional_map=out_optional,
        )

    async def run(self, gremlin: Gremlin) -> Outcome:
        state = gremlin.state
        if state is None:
            raise RuntimeError("exec stage requires gremlin.state to be initialized")
        try:
            extra_env = resolve_in_map(
                state.artifacts, self.in_map, self.in_optional_map
            )
        except ValueError as exc:
            raise Bail(f"exec {self.name}: {exc}") from exc

        pre_sha: str | None = None
        all_out = {**self.out_map, **self.out_optional_map}
        if any(Uri.is_range(v) for v in all_out.values()):
            pre_sha = snapshot_head_before(cwd=pathlib.Path(state.cwd))

        cmds = [
            _sub_artifact_paths(
                self.substitute_vars(c.rstrip(), state, extra_env),
                state.artifacts,
            )
            for c in self.options.get("cmds", [])
            if c.strip()
        ]
        bail_triggered = False
        shell_output = ""
        shell_rc = 0
        raw_timeout = self.options.get("timeout")
        timeout: float | None = float(raw_timeout) if raw_timeout is not None else None
        if cmds:
            result = await _proc.run_shell_async(
                " && ".join(cmds),
                cwd=pathlib.Path(state.cwd),
                env={**os.environ, **extra_env},
                timeout=timeout,
            )
            log_path = state.artifact_dir / f"exec-{self.name}.log"
            log_path.write_text(
                result.stdout + result.stderr or "(no output)\n", encoding="utf-8"
            )
            shell_output = (result.stdout + result.stderr).strip()
            shell_rc = result.returncode
            if result.returncode != 0:
                if result.returncode == 2 and (
                    _BAIL_KEY in self.out_map
                    or _BAIL_KEY in self.out_optional_map
                ):
                    bail_triggered = True
                else:
                    raise Bail(f"exec {self.name}: exited {result.returncode}")

        for raw_key, raw_uri_str in all_out.items():
            key = self.substitute_vars(raw_key, state, extra_env)
            key = _sub_reads(key, state.artifacts)
            optional = raw_key in self.out_optional_map
            if key == _BAIL_KEY and not bail_triggered:
                continue
            try:
                uri_str = self.substitute_vars(
                    _sub_reads(raw_uri_str, state.artifacts), state, extra_env
                )
            except MissingArtifact:
                if optional:
                    continue
                raise
            if Uri.is_range(uri_str):
                if pre_sha is None:
                    raise RuntimeError(
                        f"exec {self.name}: git://range requires pre-snapshot"
                    )
                state.artifacts.bind_git_commit_range(key, pre_sha)
            elif "://" not in uri_str:
                # Bare key (e.g. "bail", "done", "status") — store the file
                # contents if the corresponding artifact file exists.
                value: Any = uri_str
                file_path = state.artifact_dir / key
                if file_path.exists():
                    value = file_path.read_text(encoding="utf-8").strip()
                try:
                    state.artifacts.bind(key, value)
                except DuplicateArtifact:
                    if optional:
                        continue
                    raise
            else:
                uri = Uri.parse(uri_str)
                materialized = state.artifacts.resolver(uri.scheme).materialize(
                    uri_str
                )
                try:
                    state.artifacts.resolver(uri.scheme).verify_produced(materialized)
                except FileNotFoundError:
                    if optional:
                        continue
                    if key == _BAIL_KEY:
                        msg = f"exec {self.name}: exited {shell_rc}"
                        if shell_output:
                            msg += f"\n{shell_output}"
                        raise Bail(msg) from None
                    raise
                try:
                    state.artifacts.bind(key, materialized)
                except DuplicateArtifact:
                    if optional:
                        continue
                    raise

        return Done()
