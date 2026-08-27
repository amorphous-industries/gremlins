"""Tests for LoopStage termination paths and Exec stage."""

from __future__ import annotations

import asyncio
import json
from typing import TYPE_CHECKING, Any, cast

import pytest
from conftest import MockGremlin, _make_gremlin_wrapper

from gremlins.artifacts.uri import Uri
from gremlins.executor.state import State as RuntimeState
from gremlins.executor.state import StateData, build_state
from gremlins.stages.loop import LoopStage
from gremlins.stages.outcome import Bail, Done

if TYPE_CHECKING:
    from gremlins.executor.gremlin import Gremlin


def _fake_client() -> Any:
    from tests.fake_client import FakeClient

    return FakeClient(fixtures={})


def _loop_state(tmp_path: Any) -> RuntimeState:
    (tmp_path / "artifacts").mkdir(exist_ok=True)
    return build_state(
        data=StateData(),
        client=_fake_client(),
        artifact_dir=tmp_path / "artifacts",
        worktree=tmp_path,
    )


def _set_done(state: RuntimeState) -> None:
    """Write the done artifact to signal loop completion."""
    done_uri = Uri.parse("file://session/done.txt")
    state.artifacts.bind("done", done_uri)


# ---------------------------------------------------------------------------
# LoopStage termination paths
# ---------------------------------------------------------------------------


def test_loop_exhausted_bails_without_stop_condition(tmp_path):
    """No stop_when_exists and no bail → exhausts iterations then bails."""

    async def runner() -> Done:
        return Done()

    loop = LoopStage("loop", body_runners=[runner], max_iterations=3)
    with pytest.raises(Bail):
        asyncio.run(loop.run(_make_gremlin_wrapper(_loop_state(tmp_path))))


def test_loop_stops_when_stop_when_exists_artifact_is_bound(tmp_path):
    """Loop with stop_when_exists stops when the artifact is bound."""
    loop_state = _loop_state(tmp_path)
    calls: list[str] = []

    async def runner() -> Done:
        calls.append("run")
        _set_done(loop_state)
        return Done()

    loop = LoopStage(
        "loop", body_runners=[runner], max_iterations=3, stop_when_exists="done"
    )
    outcome = asyncio.run(loop.run(_make_gremlin_wrapper(loop_state)))

    assert outcome == Done()
    assert calls == ["run"]


def test_loop_cmd_failure_then_fix_then_green(tmp_path):
    """Body runs fully each iteration. Fix sets done on second try."""
    loop_state = _loop_state(tmp_path)
    attempt = {"attempt": 0, "fixed": False}

    async def check() -> Done:
        attempt["attempt"] += 1
        return Done()

    async def fix() -> Done:
        if attempt["fixed"]:
            _set_done(loop_state)
        attempt["fixed"] = True
        return Done()

    loop = LoopStage(
        "loop", body_runners=[check, fix], max_iterations=3, stop_when_exists="done"
    )
    asyncio.run(loop.run(_make_gremlin_wrapper(loop_state)))

    assert attempt["attempt"] == 2
    assert attempt["fixed"]


def test_loop_body_runs_fully_each_iteration(tmp_path):
    """All body runners execute every iteration — no conditional skipping."""
    fix_calls: list[int] = []

    async def check() -> Done:
        return Done()

    async def fix() -> Done:
        fix_calls.append(1)
        _set_done(loop_state)
        return Done()

    loop_state = _loop_state(tmp_path)
    loop = LoopStage(
        "loop", body_runners=[check, fix], max_iterations=3, stop_when_exists="done"
    )
    asyncio.run(loop.run(_make_gremlin_wrapper(loop_state)))

    # fix always runs, even when check succeeds
    assert fix_calls == [1]


def test_loop_exhausted_returns_bail(tmp_path):
    loop_state = _loop_state(tmp_path)

    async def check() -> Done:
        return Done()

    async def fix() -> Done:
        return Done()

    # No stop_when_exists → runs max_iterations then bails
    loop = LoopStage("loop", body_runners=[check, fix], max_iterations=3)
    with pytest.raises(Bail):
        asyncio.run(loop.run(_make_gremlin_wrapper(loop_state)))


def test_loop_body_fully_executes_on_final_iteration(tmp_path):
    """Body always runs fully even on the last iteration."""
    fix_calls: list[int] = []
    attempt = [0]
    loop_state = _loop_state(tmp_path)

    async def check() -> Done:
        attempt[0] += 1
        return Done()

    async def fix() -> Done:
        fix_calls.append(attempt[0])
        return Done()

    loop = LoopStage("loop", body_runners=[check, fix], max_iterations=3)
    with pytest.raises(Bail):
        asyncio.run(loop.run(_make_gremlin_wrapper(loop_state)))
    # all body stages run every iteration, including the last
    assert fix_calls == [1, 2, 3]


def test_loop_bail_propagates_immediately(tmp_path):
    """Bail raised from a body runner propagates without continuing."""

    async def bail_runner() -> Done:
        raise Bail("stage bailed: bail_class=other")

    loop = LoopStage("loop", body_runners=[bail_runner], max_iterations=3)
    with pytest.raises(Bail) as exc_info:
        asyncio.run(loop.run(_make_gremlin_wrapper(_loop_state(tmp_path))))
    assert "bail_class=other" in exc_info.value.reason


def test_loop_exhausted_emits_bail_to_state(tmp_path, make_state_dir):
    import gremlins.executor.state as state_mod

    gremlin_id = "loop-test-gr"
    state_dir = make_state_dir(gremlin_id)
    attempt = "loop-test-attempt"
    state_mod.StateData.load(gremlin_id).patch(attempt=attempt)

    (tmp_path / "artifacts").mkdir(exist_ok=True)
    loop_state = build_state(
        data=StateData(gremlin_id=gremlin_id, attempt=attempt),
        client=_fake_client(),
        artifact_dir=tmp_path / "artifacts",
        worktree=tmp_path,
    )

    async def runner() -> Done:
        return Done()

    loop = LoopStage("loop", body_runners=[runner], max_iterations=2)
    with pytest.raises(Bail):
        asyncio.run(loop.run(_make_gremlin_wrapper(loop_state)))

    bail_file = state_dir / f"bail_{attempt}.json"
    assert bail_file.exists()
    data = json.loads(bail_file.read_text())
    assert data["class"] == "other"


# ---------------------------------------------------------------------------
# stop_when_exists from YAML
# ---------------------------------------------------------------------------


def test_stop_when_exists_from_yaml(tmp_path):
    """with_dict parses stop_when_exists from YAML."""
    loop = LoopStage.with_dict(
        {
            "type": "loop",
            "stop_when_exists": "done",
            "max-iterations": "3",
            "body": [],
        }
    )
    assert loop._stop_when_exists == "done"


def test_no_stop_when_exists_defaults_to_none(tmp_path):
    """with_dict leaves stop_when_exists None when not in YAML."""
    loop = LoopStage.with_dict(
        {
            "type": "loop",
            "max-iterations": "3",
            "body": [],
        }
    )
    assert loop._stop_when_exists is None


# ---------------------------------------------------------------------------
# loop_iteration written to state.json
# ---------------------------------------------------------------------------


def test_loop_patches_loop_iteration_to_state(tmp_path, make_state_dir):
    gremlin_id = "iter-patch-test"
    state_dir = make_state_dir(gremlin_id)
    seen_iterations: list[int] = []

    (tmp_path / "artifacts").mkdir(exist_ok=True)
    loop_state = build_state(
        data=StateData(gremlin_id=gremlin_id),
        client=_fake_client(),
        artifact_dir=tmp_path / "artifacts",
        worktree=tmp_path,
    )

    async def runner() -> Done:
        data = json.loads((state_dir / "state.json").read_text())
        seen_iterations.append(int(data.get("loop_iteration") or 0))
        return Done()

    loop = LoopStage("loop", body_runners=[runner], max_iterations=3)
    with pytest.raises(Bail):
        asyncio.run(loop.run(_make_gremlin_wrapper(loop_state)))

    assert seen_iterations == [1, 2, 3]


def test_loop_unbinds_out_keys_between_iterations(tmp_path):
    """out_map keys unbound each iteration so exec can rebind to a different URI."""
    from gremlins.stages.exec import Exec

    (tmp_path / "artifacts").mkdir(exist_ok=True)
    state = _loop_state(tmp_path)

    bound_count = [0]

    async def binder() -> Done:
        uri = Uri.parse(f"file://session/out-{bound_count[0]}.txt")
        state.artifacts.bind("loop-out", uri)
        bound_count[0] += 1
        if bound_count[0] == 2:
            _set_done(state)
        return Done()

    exec_stage = Exec("stage", {}, out_map={"loop-out": "file://session/out-0.txt"})
    loop = LoopStage(
        "loop",
        body=[exec_stage],
        body_runners=[binder],
        max_iterations=3,
        stop_when_exists="done",
    )
    asyncio.run(loop.run(cast("Gremlin", MockGremlin(state))))
    assert bound_count[0] == 2


# ---------------------------------------------------------------------------
# interval option
# ---------------------------------------------------------------------------


def test_loop_interval_sleeps_between_iterations(tmp_path, monkeypatch):
    sleep_calls: list[float] = []

    async def fake_sleep(secs: float) -> None:
        sleep_calls.append(secs)

    import gremlins.stages.loop as _loop_mod

    monkeypatch.setattr(_loop_mod.asyncio, "sleep", fake_sleep)

    loop_state = _loop_state(tmp_path)
    count = [0]

    async def runner() -> Done:
        count[0] += 1
        if count[0] == 2:
            _set_done(loop_state)
        return Done()

    loop = LoopStage(
        "loop",
        body_runners=[runner],
        max_iterations=3,
        interval=5.0,
        stop_when_exists="done",
    )
    asyncio.run(loop.run(_make_gremlin_wrapper(loop_state)))

    assert count[0] == 2
    assert sleep_calls == [5.0]


def test_loop_no_interval_no_sleep(tmp_path, monkeypatch):
    sleep_calls: list[float] = []

    async def fake_sleep(secs: float) -> None:
        sleep_calls.append(secs)

    import gremlins.stages.loop as _loop_mod

    monkeypatch.setattr(_loop_mod.asyncio, "sleep", fake_sleep)

    async def runner() -> Done:
        return Done()

    loop = LoopStage("loop", body_runners=[runner], max_iterations=3)
    with pytest.raises(Bail):
        asyncio.run(loop.run(_make_gremlin_wrapper(_loop_state(tmp_path))))

    assert sleep_calls == []
