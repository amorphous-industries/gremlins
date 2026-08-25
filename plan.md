# Centralized registry accessor methods

## Context

`ArtifactRegistry` has a clean public API (`read`, `resolve`, `produced`, `bind`, `keys`, `write`, `unbind`) but 10 call sites across the codebase bypass it — reading `registry.json` directly with `json.loads`, hand-parsing URI prefixes, accessing internal `.data` / `.registry_path` attrs, misusing the stage-DSL `resolve_in_map` in orchestration code, and reading artifact files directly from the filesystem. Each bypass duplicates logic that should live in `ArtifactRegistry` and `Uri`.

This centralizes all registry parse/access logic into `ArtifactRegistry` (and `Uri` for URI-level helpers) so callers never need to know the persistence format, the URI structure, or the internal dict layout.

## Approach

Add typed accessor methods to `ArtifactRegistry` for the common queries (`get_base_sha`, `get_pr_url`, `get_file_contents`, etc.), add `merge_from` for inter-registry file-copy + binding, add a `raw_entry` method for display use, add a `from_registry_file` classmethod for loading from a known path. Add `Uri.is_range` and `Uri.parse_or_none` static helpers. Replace all 10 patterns at their call sites. No new files — only additions to `registry.py`, `uri.py`, and call-site edits.

No backwards-compat shims. No new abstractions. `resolve_in_map` stays for stage `in:` DSL evaluation — it's not touched. `registry.json` format is unchanged.

## Tasks

- [ ] Task 1: Add `raw_entry(key) -> str | None` to `ArtifactRegistry` — returns the unresolved URI string (or `None`) for display.

- [ ] Task 2: Add `get_base_sha() -> str` and `get_base_ref() -> str` to `ArtifactRegistry` — resolve `base_sha`/`base_ref` bindings and strip the `git://commit/` / `git://ref/` prefix.

- [ ] Task 3: Add `get_pr_url() -> str | None` to `ArtifactRegistry` — resolve the `pr-url` or `pr` binding and return the URL string or `None`.

- [ ] Task 4: Add `get_file_contents(key, *, default="") -> str` to `ArtifactRegistry` — for `file://session/` artifacts, read and return file contents; return `default` if unbound or not a file artifact.

- [ ] Task 5: Add `merge_from(other, *, key_prefix="", copy_files=False, dest_artifact_dir=None)` to `ArtifactRegistry` — copy file artifacts + rebind non-file URIs from another registry, with optional key prefix for disambiguation. The current `_gather_child_artifacts` logic in `stages/parallel.py` becomes a call to this.

- [ ] Task 6: Add `from_registry_file(path, *, artifact_dir, cwd=None)` classmethod to `ArtifactRegistry` — load a registry from a `registry.json` at an arbitrary path (used by `Gremlin.fork`).

- [ ] Task 7: Add `Uri.is_range(value) -> bool` static method and `Uri.parse_or_none(s) -> Uri | None` static method.

- [ ] Task 8: Replace pattern 1 (`executor/run.py:196-210`): construct an `ArtifactRegistry` early (or use `from_registry_file`) and call `get_base_sha()` / `get_base_ref()` instead of raw `json.loads` + prefix stripping.

- [ ] Task 9: Replace pattern 2 (`executor/run.py:326`): `gremlin.registry.get_pr_url() or "unknown"` instead of `resolve_in_map`.

- [ ] Task 10: Replace pattern 3 (`fleet/land.py:751-756`): `registry.get_pr_url()` instead of `resolve_in_map`.

- [ ] Task 11: Replace pattern 4 (`stages/parallel.py:_gather_child_artifacts`): load child `ArtifactRegistry` via `from_registry_file`, call `parent.merge_from(child, copy_files=True, ...)`.

- [ ] Task 12: Replace pattern 5 (`cli/artifacts.py:91-93`): use `reg.keys()` + `reg.raw_entry(k)` instead of `reg.data`. Use `Uri.parse_or_none(v)` for scheme extraction.

- [ ] Task 13: Replace pattern 6 (`cli/artifacts.py:87-88`): delete `extract_scheme` helper, use `Uri.parse_or_none(u)` inline.

- [ ] Task 14: Replace pattern 7 (`executor/gremlin.py:248-251`): use `ArtifactRegistry.from_registry_file(src_registry, artifact_dir=child_artifact_dir, cwd=child_worktree)` and persist to child dir, instead of `shutil.copy2` + constructing separately.

- [ ] Task 15: Replace pattern 8 (`fleet/land.py:379,386,519`): `registry.get_file_contents("plan")`, `.get_file_contents("spec")` instead of `os.path.join(wdir, "artifacts", "plan.md")` + `open`.

- [ ] Task 16: Replace pattern 9 (`stages/exec.py:89,142`): `Uri.is_range(v)` instead of `v == "git://range"` and `uri_str == "git://range"`.

- [ ] Task 17: Add tests in `tests/artifacts/test_registry.py` for `raw_entry`, `get_base_sha`, `get_base_ref`, `get_pr_url`, `get_file_contents`, `merge_from`, `from_registry_file`.

- [ ] Task 18: Add tests in `tests/artifacts/test_uri.py` for `Uri.is_range` and `Uri.parse_or_none`.

- [ ] Task 19: Update `tests/test_cli_artifacts.py` for the `raw_entry` API change.

- [ ] Task 20: Update `tests/test_parallel_fanin_artifacts.py` for the `merge_from` refactor.

- [ ] Task 21: Update `tests/test_launcher_input_sources.py` — replace `registry.data["key"]` assertions with `registry.raw_entry("key")` or `registry.produced("key")`.

- [ ] Task 22: Grep for additional patterns and fix any newly discovered call sites that bypass the registry API. Run `rg 'registry\.json' --type py`, `rg 'resolve_in_map.*registry' --type py`, `rg '\.data\b' --type py --glob='!test_*'`, `rg 'os\.path\.join.*"artifacts"' --type py`, `rg '== "git://range"' --type py`, `rg 'startswith\("file://' --type py`, `rg 'startswith\("git://' --type py`.

- [ ] Task 23: Run `make -j8 test` and fix any failures.

## Open questions

None — the design is fully specified in the pre-existing plan. Implementation is mechanical replacement of known patterns.
