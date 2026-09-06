import pathlib

import pytest
from _gremlins_core.discovery import resolve_pipeline_name


def test_hit_project_local(
    tmp_path: pathlib.Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    pipelines_dir = tmp_path / ".gremlins"
    pipelines_dir.mkdir(parents=True)
    (pipelines_dir / "mypipe.yaml").write_text("stages: []\n")
    monkeypatch.setenv("GREMLINS_PROJECT_ROOT", str(tmp_path))
    result = resolve_pipeline_name("mypipe")
    assert result == (pipelines_dir / "mypipe.yaml").resolve()


def test_project_overlay_wins(
    tmp_path: pathlib.Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    pipelines_dir = tmp_path / ".gremlins"
    pipelines_dir.mkdir(parents=True)
    (pipelines_dir / "local.yaml").write_text("stages: []\n")
    monkeypatch.setenv("GREMLINS_PROJECT_ROOT", str(tmp_path))
    result = resolve_pipeline_name("local")
    assert result == (pipelines_dir / "local.yaml").resolve()


def test_miss_raises_with_suggestions(
    tmp_path: pathlib.Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    pipelines_dir = tmp_path / ".gremlins"
    pipelines_dir.mkdir(parents=True)
    (pipelines_dir / "alpha.yaml").write_text("stages: []\n")
    monkeypatch.setenv("GREMLINS_PROJECT_ROOT", str(tmp_path))
    with pytest.raises(FileNotFoundError) as exc_info:
        resolve_pipeline_name("nonexistent")
    msg = str(exc_info.value)
    assert "nonexistent" in msg
    assert "alpha" in msg
