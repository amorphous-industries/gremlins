# Concurrent tool execution

## Goal
When the LLM returns multiple tool calls in a single turn, execute them concurrently via `futures::future::join_all` instead of serially `await`ing each one.

## Background
The agent loop in `openai_backend.rs` (`run_agent_loop_core`) iterates tool calls one at a time:

```rust
for tc in &tool_calls {
    let output = tools::invoke(&tc.function.name, tool_ctx, &args_json).await;
    // ...
}
```

Tool invocations within a turn are independent — they all see the same conversation state and don't mutate shared tool-context fields (each tool call gets its own cloned `args_json`). The only ordering constraint is that results must be appended to history in the same order the model emitted them, which `join_all` preserves.

This is a free latency win. A turn with 3 file reads that currently takes 3× the slowest read will take max(slowest read). No new coordination or state management needed.

## Design

- **`openai_backend.rs`:** In `run_agent_loop_core`, replace the serial `for tc in &tool_calls { tools::invoke(...).await }` with a `join_all` over a vector of futures. Collect results in order, then zip back with the origina call IDs to build `result_msgs`.
  - `ToolContext` is already `Clone`, so each future can capture its own clone.
  - Stream emissions (`emit_tool`, `emit_result`), raw-file writes, and event capture stay as they are — these happen *outside* the concurrent block, before and after the `join_all`.
  - The `nested` guard stays: stream/raw/capture are skipped entirely when `nested` is true, same as today.
- **No changes** to `tools.rs`, `subagent.rs`, `retry.rs`, or `cmd_backend.rs`. Tool implementations don't share mutable state.
- **No changes** to `DESIGN.md`, `AGENTS.md`, or any Python code.
- Follow conventions: functional, no inheritance, no new types.

## Test plan

- **`crates/gremlins/src/clients/openai_backend.rs` tests:** Add a test where the mock model issues multiple tool calls in one turn (e.g., two Reads) and assert both complete independently. The existing `loop_tool_then_text` test already covers a single-tool call — extend or clone it for multi-tool.
- **Edge cases:** A tool call that fails among successes — the error should appear in its position in the ordered result set. A mix of tool types (Read + Write + Bash) in a single turn — all execute concurrently.
- **No new test files needed.**

## Landing

- `make -j8 test` passes (Rust tests via `cargo test` included).
- Multi-tool turns in the mock-model test finish in ~1× the slowest tool, not N×.
- No regressions: existing single-tool and no-tool tests pass unchanged.