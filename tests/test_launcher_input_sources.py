"""Tests for launch-time bootstrap.source validation."""

import pathlib

import pytest

from gremlins.pipeline.bootstrap import validate_source_values
from gremlins.pipeline.inputs import InputSource, InputSources


def _sources(*items: tuple[str, list[str], bool]) -> InputSources:
    return InputSources(
        {
            name: InputSource(name=name, types=types, optional=optional)
            for name, types, optional in items
        }
    )


class TestValidateSourceValues:
    def test_string_source_accepted(self) -> None:
        validate_source_values(
            _sources(("instructions", ["string"], False)),
            {"instructions": "do the thing"},
        )

    def test_filepath_source_accepted(self, tmp_path: pathlib.Path) -> None:
        plan_file = tmp_path / "plan.md"
        plan_file.write_text("# Plan", encoding="utf-8")
        validate_source_values(
            _sources(("plan", ["filepath"], False)), {"plan": str(plan_file)}
        )

    def test_union_type_accepts_string(self) -> None:
        validate_source_values(
            _sources(("plan", ["filepath", "string"], True)), {"plan": "#123"}
        )

    def test_optional_source_absent_ok(self) -> None:
        validate_source_values(_sources(("instructions", ["string"], True)), {})

    def test_required_source_absent_raises(self) -> None:
        with pytest.raises(ValueError, match="required bootstrap.source"):
            validate_source_values(_sources(("plan", ["string"], False)), {})

    def test_filepath_only_no_file_raises(self) -> None:
        with pytest.raises(ValueError, match="expected an existing file"):
            validate_source_values(
                _sources(("plan", ["filepath"], False)),
                {"plan": "/nonexistent/plan.md"},
            )

    def test_unknown_key_in_input_values_ignored(self) -> None:
        validate_source_values(
            _sources(("plan", ["string"], True)),
            {"plan": "ref", "extra": "ignored"},
        )
