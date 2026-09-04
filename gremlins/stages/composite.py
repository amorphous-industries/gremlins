"""Shared helpers for composite stages (Loop, Sequence, Parallel)."""

from __future__ import annotations

import dataclasses
import logging
import pathlib

from _gremlins_core.config import scratch_root

from gremlins.executor.gremlin import State
from gremlins.stages.base import Stage

logger = logging.getLogger(__name__)


def child_state(
    parent: State, child: Stage, *, fan_out: bool = False, child_id: str | None = None
) -> State:
    """Derive a child State from parent."""
    client = (
        child.client
        if (child.client is not None and child.client_explicit)
        else parent.client
    )
    logger.debug(
        "child_state: parent=%s child=%s client=%s:%s fan_out=%s child_id=%s",
        id(parent),
        child.name,
        client.provider,
        client.model,
        fan_out,
        child_id,
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
    logger.debug(
        "child_state: fan-out child=%s artifact_dir=%s",
        child.name,
        artifact_dir,
    )
    return dataclasses.replace(
        parent,
        client=client,
        artifact_dir=artifact_dir,
        child_key=child.name,
    )
