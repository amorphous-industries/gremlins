"""Agent primitive stage: resolves in: artifacts, renders prompt, invokes agent, verifies out:."""

from __future__ import annotations

import pathlib
import secrets
import shutil
from typing import TYPE_CHECKING, Any, cast

from gremlins.artifacts.resolve import resolve_in_map
from gremlins.artifacts.uri import Uri
from gremlins.stages.agent_runner import run_agent
from gremlins.stages.base import Stage, get_client_from_dict
from gremlins.stages.constants import FRAMEWORK_KEYS
from gremlins.stages.outcome import Bail, Done, Outcome

if TYPE_CHECKING:
    from gremlins.executor.gremlin import Gremlin


class Agent(Stage):
    """YAML type: agent.

    in:  var_name -> registry_key   (resolved content substituted into prompt)
    out: registry_key -> uri_string (bound before run, verified after)

    Options:
        model: override the pipeline-default model for this stage.

    When out: declares a single file://session/<name> binding, the agent is
    instructed via the {out_file} prompt variable to write that file to
    <uuid-slug>_<name> in the worktree; after the agent completes, the file is
    moved to {artifact_dir}/<name>. The uuid-slug keeps sibling/parallel agents
    from colliding on the same worktree path.

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
        in_map: dict[str, str] | None = None,
        out_map: dict[str, str] | None = None,
    ) -> None:
        super().__init__(name)
        self.prompts = prompts
        self.options = options
        self.in_map = in_map or {}
        self.out_map = out_map or {}

    @classmethod
    def with_dict(cls, d: dict[str, Any], depth: int = 0) -> Agent:
        name = d.get("name") or ""
        raw_in: object = d.get("in") or {}
        raw_out: object = d.get("out") or {}
        if not isinstance(raw_in, dict):
            raise ValueError(f"stage {name!r}: 'in' must be a mapping")
        if not isinstance(raw_out, dict):
            raise ValueError(f"stage {name!r}: 'out' must be a mapping")
        for k in cast(dict[str, Any], d.get("options") or {}):
            if k in FRAMEWORK_KEYS - {"model"}:
                raise ValueError(
                    f"stage {name!r}: option key {k!r} collides with framework substitution variable"
                )
        stage = cls(
            name,
            d.get("prompt") or [],
            d.get("options") or {},
            in_map=dict(cast(dict[str, str], raw_in)),
            out_map=dict(cast(dict[str, str], raw_out)),
        )
        stage.client = get_client_from_dict(d)
        return stage

    async def run(self, gremlin: Gremlin) -> Outcome:
        state = gremlin.state
        if state is None:
            raise RuntimeError("agent stage requires gremlin.state to be initialized")
        opts = dict(self.options)
        raw_model = cast(str | None, opts.pop("model", None))

        try:
            resolved = resolve_in_map(state.artifacts, self.in_map)
        except ValueError as exc:
            raise Bail(f"agent {self.name}: {exc}") from exc

        out_map = {
            self.substitute_vars(k, state, resolved): self.substitute_vars(
                v, state, resolved
            )
            for k, v in self.out_map.items()
        }
        for key, uri_str in out_map.items():
            if not state.artifacts.produced(key):
                state.artifacts.bind(key, Uri.parse(uri_str))

        final_name = self._single_file_output(out_map)
        worktree_name = f"{secrets.token_hex(4)}_{final_name}" if final_name else None
        if worktree_name:
            resolved["out_file"] = worktree_name

        template = "\n\n".join(self.prompts).rstrip()
        prompt = self.substitute_vars(template, state, resolved)

        raw_path = state.artifact_dir / f"stream-{self.name}.jsonl"
        model = self.substitute_vars(raw_model, state, resolved) if raw_model else None
        await run_agent(
            state, prompt, label=self.name, raw_path=raw_path, model=model, **opts
        )

        if worktree_name and final_name:
            src = pathlib.Path(state.cwd) / worktree_name
            dst = state.artifact_dir / final_name
            if src.exists():
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.move(src, dst)

        for key, uri_str in out_map.items():
            uri = Uri.parse(uri_str)
            state.artifacts.resolver(uri.scheme).verify_produced(uri)

        return Done()

    @staticmethod
    def _single_file_output(out_map: dict[str, str]) -> str | None:
        """Return the file://session/<name> filename when out: declares exactly one."""
        names: list[str] = []
        for uri_str in out_map.values():
            try:
                uri = Uri.parse(uri_str)
            except ValueError:
                continue
            if uri.scheme == "file" and uri.path.startswith("session/"):
                names.append(uri.path[len("session/") :])
        return names[0] if len(names) == 1 else None
