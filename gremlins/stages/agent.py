"""Agent primitive stage: resolves in: artifacts, renders prompt, invokes agent, verifies out:."""

from __future__ import annotations

import json
import os
import secrets
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

    When out: declares file://session/<name> bindings, the agent is instructed
    via the {out_file} prompt variable (single output) or the {out_files} JSON
    mapping (multiple outputs) to write each file to
    {artifact_dir}/<uuid-slug>_<name>. After the agent completes, the slug
    prefix is stripped via rename to {artifact_dir}/<name>.

    A single-output stage is strict: verification raises if the file is
    missing or empty. Multi-output stages are best-effort — the agent may
    write any subset, so files it did not write are skipped without error
    (they stay bound but read back empty downstream).

    The uuid-slug prevents agents from accidentally reading or overwriting
    artifacts from prior stages in the same artifact directory.

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

        file_names = self._file_outputs(out_map)
        slug = secrets.token_hex(4)
        slugged = {name: f"{slug}_{name}" for name in file_names}
        ad = str(state.artifact_dir)
        if len(file_names) == 1:
            resolved["out_file"] = f"{ad}/{slugged[file_names[0]]}"
        elif len(file_names) > 1:
            resolved["out_files"] = json.dumps(
                {name: f"{ad}/{fname}" for name, fname in slugged.items()}
            )

        template = "\n\n".join(self.prompts).rstrip()
        prompt = self.substitute_vars(template, state, resolved)

        raw_path = state.artifact_dir / f"stream-{self.name}.jsonl"
        model = self.substitute_vars(raw_model, state, resolved) if raw_model else None
        await run_agent(
            state, prompt, label=self.name, raw_path=raw_path, model=model, **opts
        )

        for name in file_names:
            src = state.artifact_dir / slugged[name]
            dst = state.artifact_dir / name
            if src.exists():
                os.replace(src, dst)

        single = len(file_names) == 1
        for key, uri_str in out_map.items():
            uri = Uri.parse(uri_str)
            if not single and uri.scheme == "file" and uri.path.startswith("session/"):
                # Multi-output stages are best-effort: the agent may have
                # written only a subset of the declared files, so a missing
                # file is not an error here. It stays bound and reads back
                # empty downstream.
                continue
            state.artifacts.resolver(uri.scheme).verify_produced(uri)

        return Done()

    @staticmethod
    def _file_outputs(out_map: dict[str, str]) -> list[str]:
        """Return the file://session/<name> filenames declared in out:, in order.

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
