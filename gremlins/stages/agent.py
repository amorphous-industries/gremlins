"""Agent primitive stage: resolves interpolation artifacts, renders prompt, invokes agent, verifies bind outputs."""

from __future__ import annotations

import logging
import pathlib
import secrets
from typing import TYPE_CHECKING, Any, cast

from _gremlins_core.artifacts import Uri

from gremlins.artifacts.resolve import resolve_interpolation_map
from gremlins.stages.agent_runner import run_agent
from gremlins.stages.base import Stage, get_client_from_dict
from gremlins.stages.constants import FRAMEWORK_KEYS
from gremlins.stages.outcome import Bail, Done, Outcome

logger = logging.getLogger(__name__)

if TYPE_CHECKING:
    from gremlins.executor.gremlin import Gremlin


class Agent(Stage):
    """YAML type: agent.

    interpolation:  var_name -> artifact.registry_key   (resolved content substituted into prompt)
    bind: artifact.registry_key -> uri_string (bound before run, verified after)

    Options:
        model: override the pipeline-default model for this stage.

    When bind: declares file://session/<name> bindings, the agent is instructed
    via prompt variables named by each bind key (e.g. `{plan}`, `{review-code}`,
    `{local-review-two}`, `{pr_title}`) to write each file to
    {artifact_dir}/<uuid-slug>_<name>. The slug is bound into the artifact
    registry URI (file://session/<slug>_<name>) and never stripped, giving
    each run a unique file footprint that prevents agents from accidentally
    reading or overwriting artifacts from prior stages in the same artifact
    directory.

    A single-output stage is strict: verification raises if the file is
    missing or empty. Multi-output stages are best-effort — the agent may
    write any subset, so files it did not write are skipped without error
    (they stay bound but read back empty downstream).

    Unknown {keys} pass through unchanged (so code examples with braces work),
    but this also means typos like {plann} produce no error.
    """

    type = "agent"

    def __init__(
        self,
        name: str,
        prompts: list[str],
        options: dict[str, Any],
        *,
        interpolation_map: dict[str, str] | None = None,
        bind_map: dict[str, str] | None = None,
    ) -> None:
        super().__init__(name)
        self.prompts = prompts
        self.options = options
        self.interpolation_map = interpolation_map or {}
        self.bind_map = bind_map or {}

    @classmethod
    def with_dict(cls, d: dict[str, Any], depth: int = 0) -> Agent:
        from gremlins.stages.constants import (
            strip_artifact_prefix,
            strip_artifact_prefix_keys,
        )

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
            if k in FRAMEWORK_KEYS - {"model"}:
                raise ValueError(
                    f"stage {name!r}: option key {k!r} collides with framework substitution variable"
                )
        stage = cls(
            name,
            d.get("prompt") or [],
            d.get("options") or {},
            interpolation_map=strip_artifact_prefix(
                cast(dict[str, str], raw_interpolation), name
            ),
            bind_map=strip_artifact_prefix_keys(cast(dict[str, str], raw_bind), name),
        )
        stage.client = get_client_from_dict(d)
        if logger.isEnabledFor(logging.DEBUG):
            logger.debug(
                "Agent %r: %d prompts, %d interpolation keys, %d bind keys",
                name,
                len(stage.prompts),
                len(stage.interpolation_map),
                len(stage.bind_map),
            )
        return stage

    async def run(self, gremlin: Gremlin) -> Outcome:
        state = gremlin.state
        if state is None:
            raise RuntimeError("agent stage requires gremlin.state to be initialized")
        opts = dict(self.options)
        raw_model = cast(str | None, opts.pop("model", None))
        if logger.isEnabledFor(logging.DEBUG):
            logger.debug(
                "agent %s: running with %d prompts, model=%s, interpolation=%d keys, bind=%d keys",
                self.name,
                len(self.prompts),
                raw_model or "<default>",
                len(self.interpolation_map),
                len(self.bind_map),
            )

        try:
            resolved = resolve_interpolation_map(
                state.artifacts, self.interpolation_map
            )
        except ValueError as exc:
            raise Bail(f"agent {self.name}: {exc}") from exc

        resolved_bindings = {
            self.substitute_vars(k, state, resolved): self.substitute_vars(
                v, state, resolved
            )
            for k, v in self.bind_map.items()
        }
        file_names = self._file_outputs(resolved_bindings)
        slug = secrets.token_hex(4)
        slugged = {name: f"{slug}_{name}" for name in file_names}

        # Rewrite bind_map URIs to include the slug so the registry binds
        # the actual on-disk filename.
        slugged_out: dict[str, str] = {}
        for k, v in resolved_bindings.items():
            uri = Uri.parse(v)
            if uri.scheme == "file" and uri.path.startswith("session/"):
                name = uri.path[len("session/") :]
                slugged_out[k] = f"file://session/{slugged[name]}"
            else:
                slugged_out[k] = v

        if logger.isEnabledFor(logging.DEBUG):
            logger.debug(
                "agent %s: slug=%s, %d file outputs: %s",
                self.name,
                slug,
                len(file_names),
                list(slugged.keys()),
            )

        for key, uri_str in slugged_out.items():
            # Each run rebinds to a fresh slug (required for loop re-entry).
            # Slugs are never stripped — prior-iteration files stay on disk
            # in artifact_dir as an audit trail. For long-running chains
            # (e.g. boss) this accumulates files; intentional for now.
            if state.artifacts.produced(key):
                state.artifacts.unbind(key)
            state.artifacts.bind(key, Uri.parse(uri_str))

        ad = state.artifact_dir

        # Per-key bind variable paths so prompts can use {bind_key_name} directly.
        # NOTE: if a bind key name collides with an interpolation variable name,
        # this loop silently overwrites the interpolated content with the output
        # path. Pipeline authors should avoid naming conflicts between bind keys
        # and interpolation keys.
        for k, v in resolved_bindings.items():
            uri = Uri.parse(v)
            if uri.scheme == "file" and uri.path.startswith("session/"):
                name = uri.path[len("session/") :]
                resolved[k] = str(ad / slugged[name])

        template = "\n\n".join(self.prompts).rstrip()
        prompt = self.substitute_vars(template, state, resolved)

        # Inject workspace preamble so the agent knows its working directory
        # without wasting turns on cd /workspace sandbox errors.
        workspace_parts: list[str] = []
        if state.cwd:
            workspace_parts.append(f"Your working directory is: {state.cwd}")
        wt = str(state.worktree) if state.worktree else ""
        if wt and wt != state.cwd:
            workspace_parts.append(f"Project worktree: {wt}")
        workspace_parts.append(
            "Relevant environment variables: $GREMLINS_WORKTREE_PATH, $GREMLIN_WORKSPACE_DIR, $GREMLINS_ARTIFACT_DIR"
        )
        if workspace_parts:
            preamble = "\n".join(workspace_parts)
            prompt = preamble + "\n\n" + prompt

        raw_path = state.artifact_dir / f"stream-{self.name}.jsonl"
        model = self.substitute_vars(raw_model, state, resolved) if raw_model else None

        # Single-output stages: compute expected artifact paths for the
        # reminder loop so the agent is nudged to call Write if it forgets.
        single = len(file_names) == 1
        if single:
            expected_paths: list[pathlib.Path] = []
            for uri_str in slugged_out.values():
                uri = Uri.parse(uri_str)
                if uri.scheme == "file" and uri.path.startswith("session/"):
                    p = state.artifacts.file_resolver.path_for(uri)
                    expected_paths.append(p)
            if expected_paths:
                opts["expected_artifact_paths"] = expected_paths
                opts["artifact_reminder_count"] = 1

        await run_agent(
            state, prompt, label=self.name, raw_path=raw_path, model=model, **opts
        )

        if logger.isEnabledFor(logging.DEBUG):
            logger.debug(
                "agent %s: run completed, checking %d output bindings (multi-output keys skipped)",
                self.name,
                len(slugged_out),
            )

        for key, uri_str in slugged_out.items():
            uri = Uri.parse(uri_str)
            if not single and uri.scheme == "file" and uri.path.startswith("session/"):
                # Multi-output stages are best-effort: the agent may have
                # written only a subset of the declared files, so a missing
                # file is not an error here. It stays bound and reads back
                # empty downstream.
                if logger.isEnabledFor(logging.DEBUG):
                    logger.debug(
                        "agent %s: multi-output skip verification for %s -> %s",
                        self.name,
                        key,
                        uri_str,
                    )
                continue
            state.artifacts.resolver(uri.scheme).verify_produced(uri)

        return Done()

    @staticmethod
    def _file_outputs(out_map: dict[str, str]) -> list[str]:
        """Return the file://session/<name> filenames declared in bind:, in order.

        Rejects names containing '/' or '..' to prevent path-traversal escapes.
        """
        names: list[str] = []
        for key, uri_str in out_map.items():
            try:
                uri = Uri.parse(uri_str)
            except ValueError:
                continue
            if uri.scheme == "file" and uri.path.startswith("session/"):
                name = uri.path[len("session/") :]
                if "/" in name or ".." in name:
                    raise ValueError(
                        f"out key {key!r}: file://session/<name> must be a plain "
                        f"filename (no path separators or '..'), got {name!r}"
                    )
                names.append(name)
        return names
