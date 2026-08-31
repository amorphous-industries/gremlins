from __future__ import annotations

import logging
import os
import pathlib
import re
import time
from typing import TYPE_CHECKING, Any, cast

from gremlins.artifacts.registry import (
    ArtifactRegistry,
    DuplicateArtifact,
    MissingArtifact,
)
from gremlins.artifacts.resolve import resolve_interpolation_map
from gremlins.artifacts.schemes import snapshot_head_before
from gremlins.artifacts.uri import Uri
from gremlins.executor.env_provider import (
    EnvironmentProvider,
    RealEnvironmentProvider,
)
from gremlins.stages.base import Stage
from gremlins.stages.constants import (
    FRAMEWORK_KEYS,
    strip_artifact_prefix,
    strip_artifact_prefix_keys,
)
from gremlins.stages.outcome import Bail, Done, Outcome

logger = logging.getLogger(__name__)

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
        try:
            uri = artifacts.resolve(key)
        except MissingArtifact:
            raise MissingArtifact(key) from None
        p = artifacts.path_for(key)
        if p is None:
            raise ValueError(
                f"{{artifact:{key}}}: artifact is bound to {uri} "
                f"which is not a file://session/ path"
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
        interpolation_map: dict[str, str] | None = None,
        bind_map: dict[str, str] | None = None,
        env_provider: EnvironmentProvider | None = None,
    ) -> None:
        super().__init__(name)
        self.options = options
        self.interpolation_map = interpolation_map or {}
        self.bind_map = bind_map or {}
        self.env_provider: EnvironmentProvider = (
            env_provider or RealEnvironmentProvider()
        )

    @classmethod
    def with_dict(cls, d: dict[str, Any], depth: int = 0) -> Exec:
        name = d.get("name") or ""
        raw_interpolation: object = d.get("interpolation") or {}
        raw_bind: object = d.get("bind") or {}
        if "in" in d or "out" in d:
            raise ValueError(
                f"stage {name!r}: 'in'/'out' keys are no longer supported; "
                f"use 'interpolation'/'bind' with 'artifact.' prefix on registry keys"
            )
        if not isinstance(raw_interpolation, dict):
            raise ValueError(f"stage {name!r}: 'interpolation' must be a mapping")
        if not isinstance(raw_bind, dict):
            raise ValueError(f"stage {name!r}: 'bind' must be a mapping")
        for k in cast(dict[str, Any], d.get("options") or {}):
            if k in FRAMEWORK_KEYS:
                raise ValueError(
                    f"stage {name!r}: option key {k!r} collides with framework substitution variable"
                )
        return cls(
            name,
            d.get("options") or {},
            interpolation_map=strip_artifact_prefix(
                cast(dict[str, str], raw_interpolation), name
            ),
            bind_map=strip_artifact_prefix_keys(cast(dict[str, str], raw_bind), name),
        )

    async def run(self, gremlin: Gremlin) -> Outcome:
        state = gremlin.state
        if state is None:
            raise RuntimeError("exec stage requires gremlin.state to be initialized")
        try:
            extra_env = resolve_interpolation_map(
                state.artifacts, self.interpolation_map
            )
        except ValueError as exc:
            raise Bail(f"exec {self.name}: {exc}") from exc

        pre_sha: str | None = None
        if any(Uri.is_range(v) for v in self.bind_map.values()):
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
            joined = " && ".join(cmds)
            logger.debug(
                "exec %s: running %d command(s)",
                self.name,
                len(cmds),
            )
            for i, c in enumerate(cmds):
                logger.debug(
                    "exec %s:   cmd[%d] %s", self.name, i, c.replace("\n", "\\n")
                )
            _t0 = time.monotonic()
            result = await self.env_provider.run_shell(
                joined,
                cwd=state.cwd,
                env={**os.environ, **extra_env},
                timeout=timeout,
            )
            elapsed = time.monotonic() - _t0
            log_path = state.artifact_dir / f"exec-{self.name}.log"
            self.env_provider.write_text(
                str(log_path),
                result.stdout + result.stderr or "(no output)\n",
            )
            shell_output = (result.stdout + result.stderr).strip()
            shell_rc = result.returncode
            out_summary = (
                f" (output: {len(shell_output)} chars)" if shell_output else ""
            )
            logger.debug(
                "exec %s: done in %.2fs rc=%d%s",
                self.name,
                elapsed,
                shell_rc,
                out_summary,
            )
            if shell_output:
                logger.debug(
                    "exec %s: output written to %s (%d chars)",
                    self.name,
                    log_path,
                    len(shell_output),
                )
            if shell_rc != 0:
                if shell_output:
                    tail = shell_output[-500:]
                    logger.warning(
                        "exec %s: rc=%d, last 500 chars of output:\n%s",
                        self.name,
                        shell_rc,
                        tail,
                    )
                if shell_rc == 2 and _BAIL_KEY in self.bind_map:
                    bail_triggered = True
                else:
                    raise Bail(f"exec {self.name}: exited {shell_rc}")

        for raw_key, raw_uri_str in self.bind_map.items():
            key = self.substitute_vars(raw_key, state, extra_env)
            optional = key.endswith("?")
            if optional:
                key = key[:-1]
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
                logger.debug(
                    "exec %s: bound %s = git://range (pre_sha=%s)",
                    self.name,
                    key,
                    pre_sha,
                )
            else:
                uri = Uri.parse(uri_str)
                try:
                    state.artifacts.resolver(uri.scheme).verify_produced(uri)
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
                    state.artifacts.bind(key, uri)
                    logger.debug("exec %s: bound %s = %s", self.name, key, uri_str)
                except DuplicateArtifact:
                    if optional:
                        continue
                    raise

        return Done()
