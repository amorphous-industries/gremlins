# `gremlins/clients/`

Agent backends behind the `Client` protocol. Stages talk to one of these via
`client.run(...)` — the `Client` class is the seam tests swap out.

## Modules

- `protocol.py` — `CompletedRun` dataclass. The return type for all backend
  `run(...)` calls.
- `client.py` — `Client` class: parses `provider:model` specifiers
  (`Client.parse`), dispatches to the registered factory, and delegates
  to the backend impl.
- `registry.py` — `CLIENT_FACTORIES` dict + `register_client_factory` +
  `BYPASS_REQUIRED` set. Import-time side effects in `__init__.py` wire
  the four providers.
- `__init__.py` — registers the `openai`, `xai`, `openrouter`, and `cmd`
  factories with `CLIENT_FACTORIES` at import time. Importing the package
  is what wires the providers up.

## Backends (Rust — `_gremlins_core`)

All four providers are backed by Rust:

| Provider | Backend | Mechanism |
|---|---|---|
| `cmd` | `CmdBackend` | Spawns an arbitrary subprocess, writes prompt to stdin, parses `stream-json` events, strips optional footer |
| `openai` | `OpenAiBackend` | `rig_core::providers::openai::CompletionsClient` agent loop with tool enforcement |
| `xai` | `OpenAiBackend` | Same agent loop, pointed at xAI's OpenAI-compatible endpoint |
| `openrouter` | `OpenAiBackend` | Same agent loop, pointed at OpenRouter's endpoint |

The `cmd` provider accepts an arbitrary command template (including flags);
the model field of `Client.parse("cmd:<command>")` is
the full command string. The `openai`/`xai`/`openrouter` providers take a
model name and optionally `instructions` (a bundled prompt loaded from
`gremlins/prompts/`).

Shared Rust modules: `tools.rs` (tool definitions + enforcement),
`stream.rs` (stream parser), `config.rs` (timeout/retry constants),
`retry.rs` (exponential-backoff loop).

## Conventions

- New backends add a `RustClient` variant in `crates/gremlins-core` and
  register a factory in `gremlins/clients/__init__.py`.
- Registered providers: `openai`, `xai`, `openrouter`, `cmd`.
- The `label=` kwarg on `run(...)` is the stream-event prefix in logs and
  the `FakeClient` lookup key. Stages that re-enter the same logical
  step within one process must use distinct labels per phase so the fake's
  lookup doesn't collide.
- Never spawn a model CLI directly from a stage — go through
  `client.run(...)` so tests can substitute `FakeClient`.

## Load-bearing invariants

- `Client.parse` enforces `provider:model` shape and rejects unknown
  providers by consulting `CLIENT_FACTORIES`. Adding a provider means
  registering it in `__init__.py` + wiring a Rust backend; otherwise
  specifiers that name it fail at parse time, which is the desired
  behavior.
- The `cmd` provider's model field is the full command template. YAML
  pipelines should single-quote it:
  `default_client: 'cmd:some-tool --flag ...'`