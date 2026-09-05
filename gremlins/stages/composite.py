"""Shared helpers for composite stages (Loop, Sequence, Parallel)."""

from __future__ import annotations

import dataclasses
import pathlib
from typing import TYPE_CHECKING

from _gremlins_core.config import scratch_root

from gremlins.stages.base import Stage

if TYPE_CHECKING:
    from gremlins.executor.gremlin import State


def child_state(
    parent: State, child: Stage, *, fan_out: bool = False, child_id: str | None = None
) -> State:
    """Derive a child State from parent."""
    client = (
        child.client
        if (child.client is not None and child.client_explicit)
        else parent.client
    )

    if not fan_out:
        new_state = dataclasses.replace(parent, client=client)
        if str(client) != new_state.data.client:
            new_state.data.patch(client=str(client))
        return new_state
    if child_id:
        artifact_dir = pathlib.Path(scratch_root(child_id)) / "artifacts"
        artifact_dir.mkdir(parents=True, exist_ok=True)
    else:
        artifact_dir = parent.artifact_dir / child.name
        artifact_dir.mkdir(parents=True, exist_ok=True)
    return dataclasses.replace(
        parent,
        client=client,
        artifact_dir=artifact_dir,
        child_key=child.name,
    )
