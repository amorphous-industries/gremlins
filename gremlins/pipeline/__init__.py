from __future__ import annotations

import dataclasses
import importlib
import pathlib
from typing import TYPE_CHECKING, Any, cast

from gremlins.clients.client import Client
from gremlins.pipeline.bootstrap import Bootstrap

if TYPE_CHECKING:
    from gremlins.stages.base import Stage
    from gremlins.stages.exec import Exec

GREMLINS_PREFIX = "gremlins:"


def _fill_stage_clients(stages: list[Stage], default: Client) -> None:
    for stage in stages:
        stage.client = stage.client or default
        body = getattr(stage, "body", [])
        if body:
            _fill_stage_clients(body, default)


@dataclasses.dataclass
class Pipeline:
    name: str
    path: pathlib.Path
    stages: list[Stage]
    default_client: Client | None = None
    base_ref: str = "current"
    bootstrap: Bootstrap = dataclasses.field(default_factory=Bootstrap)
    land: Exec | None = None
    github_integration: bool = False

    def uses_loop_handoff(self) -> bool:
        first = self.stages[0] if self.stages else None
        return (
            first is not None
            and first.type == "loop"
            and any(b.name == "handoff" for b in (first.body or []))
        )

    @classmethod
    def from_yaml(
        cls, path: pathlib.Path, *, default_client_override: str | None = None
    ) -> Pipeline:
        importlib.import_module("gremlins.clients")

        from gremlins.pipeline.loader import check_duplicate_producers, parse_stages
        from gremlins.pipeline.preprocess import expand_pipeline

        path = path.resolve()
        if not path.exists():
            raise FileNotFoundError(f"pipeline file not found: {path}")

        raw = expand_pipeline(path)
        pipeline_name = path.stem

        default_client: Client | None = None
        default_client_raw = raw.get("default_client")
        if default_client_raw is not None:
            if not isinstance(default_client_raw, str):
                raise ValueError(
                    f"default_client must be a string, got {type(default_client_raw)!r}"
                )
            default_client = Client.parse(default_client_raw)

        base_ref_raw = raw.get("base_ref")
        if base_ref_raw is not None:
            if not isinstance(base_ref_raw, str) or not base_ref_raw.strip():
                raise ValueError("base_ref must be a non-empty string")
            pipeline_base_ref = base_ref_raw.strip()
        else:
            pipeline_base_ref = "current"

        github_integration = bool(raw.get("github_integration", False))

        from gremlins.stages.exec import Exec

        if "inputs" in raw:
            raise ValueError(
                "'inputs' is not a valid pipeline key; declare CLI arguments under bootstrap.source"
            )

        stages = parse_stages(cast(list[dict[str, Any]], raw.get("stages") or []))
        bootstrap = Bootstrap.from_yaml(raw.get("bootstrap"))

        land_stage: Exec | None = None
        land_raw = raw.get("land")
        if land_raw is not None:
            if not isinstance(land_raw, dict):
                raise ValueError("'land' must be a mapping")
            land_stage = Exec.with_dict({"name": "land", **land_raw})

        check_duplicate_producers(stages, extra_out=bootstrap.cli_out)

        if default_client is None and default_client_override is not None:
            default_client = Client.parse(default_client_override)

        if default_client is None:
            raise ValueError(
                "pipeline is missing 'default_client' — set a 'default_client' in the pipeline "
                "YAML or pass --client on the command line"
            )
        _fill_stage_clients(stages, default_client)

        return cls(
            name=pipeline_name,
            path=path,
            stages=stages,
            default_client=default_client,
            base_ref=pipeline_base_ref,
            bootstrap=bootstrap,
            land=land_stage,
            github_integration=github_integration,
        )
