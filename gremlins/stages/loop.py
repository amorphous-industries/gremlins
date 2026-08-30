"""LoopStage: iterate body runners until termination predicate or max iterations."""

from __future__ import annotations

import asyncio
import logging
from collections.abc import Awaitable, Callable
from typing import TYPE_CHECKING, Any, cast

from gremlins.artifacts.registry import ArtifactRegistry
from gremlins.stages.base import Stage, get_client_from_dict
from gremlins.stages.composite import child_state as _child_state
from gremlins.stages.outcome import Bail, Done, Outcome

if TYPE_CHECKING:
    from gremlins.executor.gremlin import Gremlin

logger = logging.getLogger(__name__)

_BAIL_KEY = "bail"


def _is_bail_set(artifacts: ArtifactRegistry) -> bool:
    return artifacts.produced(_BAIL_KEY)


def _do_bail(gremlin: Gremlin, artifacts: ArtifactRegistry) -> None:
    if gremlin.state is None:
        raise RuntimeError("gremlin.state is required for _do_bail")
    reason = str(artifacts.read(_BAIL_KEY)).strip()
    gremlin.state.record_bail(reason)
    raise Bail(reason)


class LoopStage(Stage):
    """Iterate body stages until max_iterations or a stop condition is met.

    Body stages execute in order every iteration. After each full body run:
    - if the bail artifact is set, the loop raises Bail
    - if the stop_when_exists artifact is bound, the loop returns Done
    - if max_iterations is reached without stopping, the loop raises Bail

    The pipeline YAML defines the stopping condition explicitly via stop_when_exists.

    Resume granularity: resuming targets the loop by name; resuming
    restarts from iteration 1, picking up file-based state from artifact_dir.
    """

    type = "loop"

    def __init__(
        self,
        name: str,
        *,
        body: list[Stage] | None = None,
        body_runners: list[Callable[[], Awaitable[Outcome]]] | None = None,
        max_iterations: int,
        stop_when_exists: str | None = None,
        interval: float | None = None,
    ) -> None:
        super().__init__(name)
        self.body = body or []
        for c in self.body:
            c.path = f"{name}/{c.name}"
        if max_iterations < 1:
            raise ValueError(
                f"LoopStage {self.name!r}: max_iterations must be >= 1, got {max_iterations}"
            )
        self._body_runners = body_runners
        self._max_iterations = max_iterations
        self._stop_when_exists = stop_when_exists
        self._interval = interval

    @classmethod
    def with_dict(cls, d: dict[str, Any], depth: int = 0) -> LoopStage:
        from gremlins.pipeline.loader import parse_stages

        name = d.get("name") or ""
        raw_options: object = d.get("options") or {}
        if not isinstance(raw_options, dict):
            raise ValueError(f"stage {name!r}: 'options' must be a mapping")
        options = cast(dict[str, Any], raw_options)
        max_iterations: int = int(
            d.get("max-iterations") or options.get("max_iterations", 3)
        )
        raw_interval = options.get("interval")
        interval: float | None = (
            float(raw_interval) if raw_interval is not None else None
        )
        stop_when_exists: str | None = d.get("stop_when_exists")

        raw_children: object = d.get("body") or []
        if not isinstance(raw_children, list):
            raise ValueError(f"stage {name!r}: 'body' must be a list")

        body = parse_stages(cast(list[dict[str, Any]], raw_children), depth=depth)
        stage = cls(
            name,
            body=body,
            max_iterations=max_iterations,
            stop_when_exists=stop_when_exists,
            interval=interval,
        )
        client = get_client_from_dict(d)
        stage.client = client
        stage.client_explicit = client is not None
        return stage

    def _build_runners(
        self, gremlin: Gremlin
    ) -> list[Callable[[], Awaitable[Outcome]]]:
        if gremlin.state is None:
            raise RuntimeError("gremlin.state is required for _build_runners")
        state = gremlin.state
        result: list[Callable[[], Awaitable[Outcome]]] = []
        for child in self.body:
            cs = _child_state(state, child)
            base: Callable[[], Awaitable[Any]] = cs.make_runner(
                child, gremlin, scope=self.body, record_stage=False
            )
            name = child.name

            async def _tracked(
                r: Callable[[], Awaitable[Any]] = base, n: str = name
            ) -> Outcome:
                state.data.patch(active_children=[n])
                try:
                    return cast(Outcome, await r())
                finally:
                    state.data.patch(_delete=("active_children",))

            result.append(cast(Callable[[], Awaitable[Outcome]], _tracked))
        return result

    async def run(self, gremlin: Gremlin) -> Outcome:
        if gremlin.state is None:
            raise RuntimeError("gremlin.state is required for LoopStage")
        for iteration in range(1, self._max_iterations + 1):
            gremlin.state.record_state_field(loop_iteration=iteration)
            gremlin.state.artifacts.unbind(_BAIL_KEY)
            for child in self.body:
                for raw_key in getattr(child, "bind_map", {}):
                    key = raw_key.removesuffix("?")
                    gremlin.state.artifacts.unbind(key)
            runners = (
                self._body_runners
                if self._body_runners is not None
                else self._build_runners(gremlin)
            )
            for runner in runners:
                await runner()

            if _is_bail_set(gremlin.state.artifacts):
                _do_bail(gremlin, gremlin.state.artifacts)

            if self._stop_when_exists is not None and gremlin.state.artifacts.produced(
                self._stop_when_exists
            ):
                return Done()

            if iteration == self._max_iterations:
                gremlin.state.record_bail(
                    f"loop exhausted {self._max_iterations} iterations"
                )
                raise Bail(f"loop exhausted {self._max_iterations} iterations")

            if self._interval is not None:
                await asyncio.sleep(self._interval)

        # All loop paths above either return Done() or raise Bail.
        raise RuntimeError(
            f"LoopStage.run() fell through — "
            f"max_iterations={self._max_iterations}, "
            f"stop_when_exists={self._stop_when_exists!r}"
        )
