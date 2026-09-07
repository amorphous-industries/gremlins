# `gremlins/artifacts/`

Artifact registry and URI model. The registry is wired into runs via
`state.artifacts` (an `ArtifactRegistry` instance constructed in
`gremlins/executor/gremlin.py` and stored on `State`). This wiring
applies to the main `Gremlin` executor path; subprocess child paths
(`spawn/child.py`) constructs `State` without artifacts.

## Public surface

```python
from gremlins.artifacts.registry import ArtifactRegistry, MissingArtifact
from _gremlins_core.artifacts import Uri
```

## URI schemes

| Scheme | Example | Description |
|---|---|---|
| `file://session/<name>` | `file://session/handoff-1.md` | File in the run's session dir |
| `git://range/<base>..<head>` | `git://range/abc123..def456` | Commit range (SHAs) |
| `git://ref/<name>` | `git://ref/main` | Git ref name (string) |
| `git://commit/<sha>` | `git://commit/abc123` | Single commit SHA (string) |

## Registry API

```python
r = ArtifactRegistry(artifact_dir=state.artifact_dir)

# Store a URI pointer (auto-resolved on read)
r.register(Uri.parse("artifact://plan"))

r.is_registered("artifact://plan")       # True
r.data_uri("artifact://plan")           # filesystem path (str)
r.exists("artifact://plan")             # True (file exists on disk)
r.content("artifact://plan")            # file content as str
r.data_uri("some_other_key")            # arbitrary stored value
r.keys()                                # iterable of bound keys
```

`data_uri()` returns the raw stored value for a key (which may be a filesystem
path string, dict, or other JSON-compatible value).  Raises `MissingArtifact(key)`
if the key is not bound.

`content()` reads file content from a registered artifact, optionally traversing
a JSON path for structured data.

In normal pipeline runs `state.artifacts` is already constructed — use it directly
rather than constructing a new registry.

## Values and JSON

All values stored in the registry are JSON-serializable by construction (paths and URI
strings). Producers and consumers own their pairwise contracts — the registry enforces
nothing beyond serializability.

URI strings stored as values (e.g. ``opaque://pr/42``) are stored
as-is. Consumers access them via `data_uri()` or `content()`.

## Data access

- `data_uri(key)` returns the raw stored value (filesystem path, dict, string,
  etc.). Raises `MissingArtifact` if unbound.
- `content(uri_str, json_path=None)` reads file content from a registered
  artifact, optionally traversing a JSON path for nested fields.
- `exists(uri)` returns True if the artifact is registered and (for file
  artifacts) the backing file exists with non-zero size.
- `is_registered(key)` returns True if the key is bound in the registry
  (no filesystem check).

## `{read:KEY}` URI substitution in `out:` maps

Any `out:` URI value may contain `{read:KEY}` tokens. Before the URI is parsed, each
token is replaced with the stripped content of the already-bound artifact at `KEY`:

```yaml
out:
  pr-number: file://session/pr-number.txt   # bound first
  pr: opaque://pr/{read:pr-number}              # reads pr-number, expands to opaque://pr/42
```

The referenced key must appear **earlier** in the `out:` map; forward references raise
`MissingArtifact`. Only `file://session/...` artifacts (resolving to a string) are
supported — passing a non-string artifact raises `TypeError`.

## Registry persistence

Bindings are atomically persisted to `artifact_dir.parent / "registry.json"`
so they survive process restart. On construction, any existing file at that
path is pre-loaded so resumed runs see prior bindings.

## Rehydration: base_ref_sha and base_ref on resume

`base_ref_sha` is written at launch time as a file artifact containing
`git://commit/<revspec>` under the key `"artifact://base_sha"`.  The value is a git
revspec — either a 40-char SHA (normal branch launch) or a symbolic ref
like `pull/N/head` (PR-mode launch); both are accepted by git commands that
consume it.  `run.py` reads the file content via `registry.content()`, not
from `registry.json` directly, before calling
`Gremlin.initialize_with_runtime()` so the worktree can be created on first
start.  On resume the worktree already exists (`workdir` is set in
`state.json`), so `base_ref_sha` is not re-used by `setup_workdir`.
The binding in `registry.json` is a filesystem path pointing to the
artifact file — that file is the authoritative source for the value.

`base_ref` (the symbolic ref name, e.g. `main`) is written at launch time as
a file artifact containing `git://ref/<name>` under the key `"artifact://base_ref"`.
`run.py` reads this via `registry.content()` and passes it to
`Gremlin.initialize_with_runtime(base_ref=...)` which threads it through
`build_state` into `State.base_ref`.  Recipes access it via the
`{base_ref}` substitution variable.
