"""End-to-end test: YAML pipeline with type: agent reads one artifact and writes one.

The plan stage that uses Pipeline.from_yaml requires prompt entries to be file paths
(the preprocessor resolves them). Here we test via parse_stages() with pre-expanded
prompts, which is what the runtime sees after preprocessing — same execution path.
"""

from __future__ import annotations

import asyncio
import pathlib
import re

from conftest import MINIMAL_EVENTS, MockGremlin

from gremlins.artifacts.registry import ArtifactRegistry
from gremlins.artifacts.uri import Uri
from gremlins.executor.state import StateData, build_state
from gremlins.pipeline.loader import parse_stages
from gremlins.stages.outcome import Done
from tests.fake_client import FakeClient


def test_agent_stage_e2e_reads_artifact_and_writes_output(tmp_path):
    """Full stack: parse from dict → run with registry → verify produced."""
    # parse_stages() receives pre-expanded prompt lists (post-preprocessing)
    raw = [
        {
            "name": "summarise",
            "type": "agent",
            "prompt": ["Summarise the following and write to `{summary}`:\n\n{src}"],
            "interpolation": {"src": "artifact.source-doc"},
            "bind": {"artifact.summary": "file://session/summary.md"},
        }
    ]
    stages = parse_stages(raw)
    assert len(stages) == 1
    stage = stages[0]
    assert stage.type == "agent"
    assert stage.name == "summarise"

    # Bind the source document in the registry
    src_file = tmp_path / "source.md"
    src_file.write_bytes(b"# Hello\nWorld")
    registry = ArtifactRegistry(tmp_path, cwd=tmp_path)
    registry.bind("source-doc", Uri.parse("file://session/source.md"))

    # Client writes the expected output file when called — extract the
    # slugged path from {summary} in the prompt.

    class WritingClient(FakeClient):
        async def run(self, prompt, *, label, **kwargs):
            m = re.search(r"`(\S*[0-9a-f]+_summary\.md)`", prompt)
            assert m, prompt
            p = pathlib.Path(m.group(1))
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text("# Summary\nHello World", encoding="utf-8")
            return await super().run(prompt, label=label, **kwargs)

    client = WritingClient(fixtures={"summarise": MINIMAL_EVENTS})
    state = build_state(
        data=StateData(),
        client=client,
        artifact_dir=tmp_path,
        worktree=tmp_path,
        artifacts=registry,
    )

    gremlin = MockGremlin(state=state)
    result = asyncio.run(stage.run(gremlin))

    assert isinstance(result, Done)
    # Source content was substituted into the prompt
    assert "# Hello" in client.calls[0].prompt
    # Output artifact is bound in the registry
    assert registry.produced("summary")
    # Output file exists (at slugged path)
    assert len(list(tmp_path.glob("*_summary.md"))) == 1


def test_agent_parse_stages_registers_type():
    """Confirm the 'agent' type is recognised by the pipeline loader."""
    raw = [
        {
            "name": "my-stage",
            "type": "agent",
            "prompt": ["Do the thing"],
        }
    ]
    stages = parse_stages(raw)
    assert stages[0].type == "agent"
