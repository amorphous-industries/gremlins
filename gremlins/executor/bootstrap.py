"""Run bootstrap commands in a worktree before pipeline stages begin.

Used at gremlin launch and in parallel child subprocesses so that
every fresh worktree gets its dev environment (venv, etc.) set up.

Supports gremlins: DSL commands in launch_cmds:
  gremlins:bind_artifact(<source_key>, <artifact_key>, <uri_template>)
    Resolves a bootstrap source value (GitHub issue ref, filepath, or inline text)
    and binds it as an artifact in the registry.
"""

from __future__ import annotations

import json
import logging
import os
import pathlib
import re
import shutil
from collections.abc import Mapping
from typing import TYPE_CHECKING, Any

from gremlins.artifacts.uri import Uri
from gremlins.pipeline.bootstrap import (
    Bootstrap,
    source_env,
    substitute_bootstrap_vars,
    validate_source_values,
)
from gremlins.utils import proc

if TYPE_CHECKING:
    from gremlins.executor.gremlin import Gremlin

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# gremlins: DSL command infrastructure
# ---------------------------------------------------------------------------

_GREMLINS_CMD_RE = re.compile(r"gremlins:([a-z_]+)\(([^)]*)\)")


async def run_bootstrap(
    cmds: list[str],
    cwd: pathlib.Path,
    extra_env: dict[str, str] | None = None,
) -> None:
    """Run shell commands in cwd. Non-zero exit raises RuntimeError."""
    cmds = [c.rstrip() for c in cmds if c.strip()]
    if not cmds:
        return
    env = dict(os.environ)
    env["GREMLINS_BOOTSTRAP_CWD"] = str(cwd)
    if extra_env:
        env.update(extra_env)
    result = await proc.run_shell_async(" && ".join(cmds), cwd=cwd, env=env)
    if result.returncode != 0:
        err = (result.stderr or result.stdout).strip()
        logger.error("bootstrap failed (exit %d): %s", result.returncode, err[:2000])
        raise RuntimeError(f"bootstrap failed (exit {result.returncode}): {err[:500]}")
    logger.info("bootstrap ok")


def _parse_gremlins_command(raw: str) -> tuple[str, list[str]] | None:
    """Parse a gremlins: DSL command from a launch_cmd string.

    Returns (command_name, [arg1, arg2, ...]) on success, None for plain shell commands.

    Syntax: gremlins:<name>(<arg1>, <arg2>, ...)
    Arguments are split on commas (outside parens) and stripped.  Quoted strings
    have their surrounding quotes removed.
    """
    match = _GREMLINS_CMD_RE.match(raw)
    if not match:
        return None
    cmd_name = match.group(1)
    args_raw = match.group(2)
    rest = raw[match.end() :]
    if rest.strip():
        # Trailing content after the closing paren — not a valid DSL command
        return None
    args = _split_dsl_args(args_raw)
    return cmd_name, args


def _split_dsl_args(args_raw: str) -> list[str]:
    """Split comma-separated DSL arguments, handling quotes."""
    parts: list[str] = []
    current: list[str] = []
    in_quote: str | None = None
    for ch in args_raw:
        if in_quote:
            if ch == in_quote:
                in_quote = None
            else:
                current.append(ch)
        elif ch in ('"', "'"):
            in_quote = ch
        elif ch == ",":
            parts.append("".join(current).strip())
            current = []
        else:
            current.append(ch)
    tail = "".join(current).strip()
    if tail:
        parts.append(tail)
    return parts


def _parse_bind_artifact_args(
    args: list[str],
) -> tuple[str, str, str]:
    """Validate and unpack bind_artifact arguments.

    Returns (source_key, artifact_key, uri_template).
    """
    if len(args) < 3:
        raise ValueError(
            f"bind_artifact requires 3 arguments (source_key, artifact_key, uri), got {len(args)}"
        )
    source_key = args[0]
    artifact_key = args[1]
    uri_template = args[2]
    if not source_key:
        raise ValueError("bind_artifact: source_key must be non-empty")
    if not artifact_key:
        raise ValueError("bind_artifact: artifact_key must be non-empty")
    if not uri_template:
        raise ValueError("bind_artifact: uri must be non-empty")
    return source_key, artifact_key, uri_template


def _execute_bind_artifact(
    source_key: str,
    artifact_key: str,
    uri_template: str,
    *,
    stage_inputs: Mapping[str, Any],
    gremlin: Gremlin,
) -> None:
    """Resolve a source value and bind it as an artifact.

    Source value resolution:
    - Empty / missing → no-op (optional source)
    - GitHub issue ref (#N or owner/repo#N) → downloads issue body via gh CLI
    - Existing filepath → copies the file to artifact_dir
    - Inline text → written directly

    After writing, the artifact is bound in the registry.
    """
    value = stage_inputs.get(source_key)
    if not value:
        return  # optional source, nothing to bind
    value_str = str(value)

    # Resolve URI template against artifact_dir to get the output path
    uri = Uri.parse(uri_template)
    resolver = gremlin.registry.file_resolver
    dest_path = resolver.path_for(uri)

    # GitHub issue refs are #N or owner/repo#N; requires # prefix so bare
    # numbers like "42" are treated as inline text, not issue references.
    gh_match = re.match(r"^(?:([^/#]+/[^#]+))?#(\d+)$", value_str)
    if gh_match:
        gh_repo = gh_match.group(1)
        gh_issue_num = gh_match.group(2)
        _fetch_github_issue(
            gh_repo, gh_issue_num, dest_path, source_key, artifact_key, uri, gremlin
        )
        return

    if os.path.isfile(value_str):
        dest_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(value_str, dest_path)
    else:
        dest_path.parent.mkdir(parents=True, exist_ok=True)
        dest_path.write_text(value_str, encoding="utf-8")

    gremlin.registry.bind(artifact_key, uri)


def _fetch_github_issue(
    gh_repo: str | None,
    gh_issue_num: str,
    dest_path: pathlib.Path,
    source_key: str,
    artifact_key: str,
    uri: Uri,
    gremlin: Gremlin,
) -> None:
    """Download a GitHub issue body and bind it as an artifact."""
    repo_flag = ["--repo", gh_repo] if gh_repo else []
    result = proc.run(
        ["gh", "issue", "view", gh_issue_num, *repo_flag, "--json", "body,title,number"],
        check=True,
        timeout=30,
    )
    data = json.loads(result.stdout)
    body = data["body"]
    title = data.get("title", "")
    number = data.get("number", "")

    if not body.startswith("# "):
        body = f"# {title}\n\n{body}"

    dest_path.parent.mkdir(parents=True, exist_ok=True)
    dest_path.write_text(body, encoding="utf-8")

    # Bind primary artifact using the artifact_key from the DSL command
    gremlin.registry.bind(artifact_key, uri)

    # Bind companion artifact: source-key -> issue number
    issue_num_name = f"{source_key}-source-issue-number"
    issue_num_dest = dest_path.parent / f"{issue_num_name}.txt"
    issue_num_dest.write_text(str(number), encoding="utf-8")
    gremlin.registry.bind(
        issue_num_name, Uri.parse(f"file://session/{issue_num_name}.txt")
    )


_DSL_DISPATCH: dict[str, object] = {
    "bind_artifact": _execute_bind_artifact,
}


async def _run_dsl_command(
    cmd_name: str,
    args: list[str],
    *,
    stage_inputs: Mapping[str, Any],
    gremlin: Gremlin,
) -> None:
    """Dispatch a parsed gremlins: DSL command to its handler."""
    handler = _DSL_DISPATCH.get(cmd_name)
    if handler is None:
        raise ValueError(
            f"unknown gremlins: command {cmd_name!r}; "
            f"known: {', '.join(sorted(_DSL_DISPATCH))}"
        )
    if cmd_name == "bind_artifact":
        source_key, artifact_key, uri_template = _parse_bind_artifact_args(args)
        _execute_bind_artifact(
            source_key, artifact_key, uri_template,
            stage_inputs=stage_inputs,
            gremlin=gremlin,
        )
    else:
        raise ValueError(f"unhandled DSL command: {cmd_name!r}")


async def run_pipeline_bootstrap(
    bootstrap: Bootstrap,
    *,
    cwd: pathlib.Path,
    artifact_dir: pathlib.Path,
    stage_inputs: Mapping[str, Any],
    gremlin: Gremlin,
    include_launch: bool,
) -> None:
    """Run worktree cmds, then (main first start only) launch_cmds and cli_out.

    Launch_cmds entries starting with ``gremlins:`` are parsed as DSL commands
    and executed inline; everything else runs as shell commands joined with ``&&``.
    """
    if bootstrap.cmds:
        await run_bootstrap(bootstrap.cmds, cwd)
    if not include_launch:
        return
    validate_source_values(bootstrap.source, stage_inputs)
    if bootstrap.launch_cmds:
        env = source_env(bootstrap.source, stage_inputs)
        shell_cmds: list[str] = []
        for c in bootstrap.launch_cmds:
            parsed = _parse_gremlins_command(c)
            if parsed:
                cmd_name, args = parsed
                await _run_dsl_command(
                    cmd_name, args,
                    stage_inputs=stage_inputs,
                    gremlin=gremlin,
                )
            else:
                shell_cmds.append(
                    substitute_bootstrap_vars(c, artifact_dir=artifact_dir, cwd=cwd)
                )
        if shell_cmds:
            await run_bootstrap(shell_cmds, cwd, extra_env=env)
    if bootstrap.cli_out:
        from gremlins.stages.exec import Exec

        binder = Exec("bootstrap", {}, out_map=dict(bootstrap.cli_out))
        await binder.run(gremlin)
