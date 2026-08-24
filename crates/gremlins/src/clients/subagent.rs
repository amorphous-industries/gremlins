use std::path::PathBuf;
use std::sync::Arc;

use rig_core::completion::CompletionModel;

use super::tools::{self, ToolContext};

const MAX_DEPTH: u32 = 3;

/// Build a subagent runner closure. Called once per backend before the agent loop.
/// The returned closure captures the model, instructions, tool filter, cancel token,
/// context prefix, and the original `ToolContext` — everything needed to run
/// a nested agent loop.
///
/// Recursive subagents are supported: the runner injects itself into the
/// sub-context's `subagent_fn` so that deeper nesting can invoke subagent
/// again (bounded by `MAX_DEPTH` via a shared atomic counter).
#[allow(clippy::too_many_arguments)]
pub(crate) fn make_runner<M: CompletionModel + Clone + Send + Sync + 'static>(
    model: M,
    instructions: String,
    tool_filter: Option<Vec<String>>,
    cancel: Arc<super::openai_backend::CancelToken>,
    ctx: ToolContext,
    prefix: String,
    idle_timeout: f64,
    max_turns: usize,
) -> tools::SubagentFn {
    // Depth counter shared across all nested subagent invocations from this parent.
    let depth = Arc::new(std::sync::atomic::AtomicU32::new(0));
    // Late-init cell so the closure can inject itself into sub-contexts.
    let self_cell: Arc<std::sync::Mutex<Option<tools::SubagentFn>>> =
        Arc::new(std::sync::Mutex::new(None));

    let runner: tools::SubagentFn = Arc::new({
        let self_cell = self_cell.clone();
        move |task: String, cwd: Option<PathBuf>| {
            let model = model.clone();
            let instructions = instructions.clone();
            let tool_filter = tool_filter.clone();
            let cancel = cancel.clone();
            let mut sub_ctx = ctx.clone();
            let prefix = prefix.clone();
            let depth = depth.clone();
            let self_cell = self_cell.clone();

            if let Some(cwd) = cwd {
                sub_ctx.cwd = Some(cwd);
            }
            // Inject self so recursive subagents can invoke subagent again.
            sub_ctx.subagent_fn = self_cell.lock().unwrap().clone();

            Box::pin(async move {
                let current = depth.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if current >= MAX_DEPTH {
                    depth.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    return format!("Error: subagent max depth ({MAX_DEPTH}) exceeded");
                }

                let result = crate::clients::openai_backend::run_agent_loop_nested(
                    &model,
                    &task,
                    &sub_ctx,
                    &cancel,
                    &instructions,
                    tool_filter.as_deref(),
                    &prefix,
                    idle_timeout,
                    max_turns,
                )
                .await;

                depth.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                match result {
                    Ok(completed) => completed.text_result.unwrap_or_default(),
                    Err(e) => format!("Subagent error: {e}"),
                }
            })
        }
    });

    // Store runner so the closure can inject it into sub-contexts.
    *self_cell.lock().unwrap() = Some(runner.clone());
    runner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn make_runner_invokes_and_sets_subagent_fn() {
        let dir = std::env::temp_dir().join(format!(
            "gremlins-sub-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let ctx = ToolContext {
            cwd: Some(dir.clone()),
            extra_env: None,
            worktree_root: dir.clone(),
            audit_log: None,
            allowed_tools: None,
            subagent_fn: None,
        };

        // Model that returns a single text response in one turn.
        let model = rig_core::test_utils::MockCompletionModel::from_stream_turns([
            vec![
                rig_core::test_utils::MockStreamEvent::text("nested reply"),
                rig_core::test_utils::MockStreamEvent::final_response_with_default_usage(),
            ],
        ]);

        let cancel = super::super::openai_backend::CancelToken::new();
        let runner = make_runner(
            model,
            "be helpful".into(),
            None,
            cancel,
            ctx,
            String::new(),
            5.0,
            10,
        );

        // First invocation: depth 0 < 3, should succeed.
        let output = runner("first call".into(), None).await;
        assert!(
            !output.contains("max depth"),
            "depth 0 should not hit guard, got: {output}"
        );
    }

    /// A model whose stream never resolves — used to keep subagent calls
    /// in-flight so the depth counter accumulates for concurrent testing.
    #[derive(Clone)]
    struct PendingModel;

    impl rig_core::completion::CompletionModel for PendingModel {
        type Response = rig_core::test_utils::MockResponse;
        type StreamingResponse = rig_core::test_utils::MockResponse;
        type Client = ();

        fn make(_: &Self::Client, _: impl Into<String>) -> Self {
            Self
        }

        async fn completion(
            &self,
            _: rig_core::completion::CompletionRequest,
        ) -> Result<
            rig_core::completion::CompletionResponse<Self::Response>,
            rig_core::completion::CompletionError,
        > {
            Err(rig_core::completion::CompletionError::ProviderError(
                "unused".into(),
            ))
        }

        async fn stream(
            &self,
            _: rig_core::completion::CompletionRequest,
        ) -> Result<
            rig_core::streaming::StreamingCompletionResponse<Self::StreamingResponse>,
            rig_core::completion::CompletionError,
        > {
            let s: rig_core::streaming::StreamingResult<Self::StreamingResponse> =
                Box::pin(futures::stream::pending());
            Ok(rig_core::streaming::StreamingCompletionResponse::stream(s))
        }
    }

    #[tokio::test]
    async fn make_runner_depth_guard_rejects_concurrent() {
        let dir = std::env::temp_dir().join(format!(
            "gremlins-sub-depth-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let ctx = ToolContext {
            cwd: Some(dir.clone()),
            extra_env: None,
            worktree_root: dir.clone(),
            audit_log: None,
            allowed_tools: None,
            subagent_fn: None,
        };

        // Use a model that hangs forever (pending stream) so concurrent
        // calls stay in-flight and the depth counter accumulates.
        let model = PendingModel;
        let cancel = super::super::openai_backend::CancelToken::new();
        let runner = make_runner(
            model,
            "instructions".into(),
            None,
            cancel,
            ctx,
            String::new(),
            0.5,
            10,
        );

        // Spawn three concurrent calls (they'll enter the loop and sit on
        // the pending stream). The fourth call should be rejected.
        let r0 = tokio::spawn(runner.clone()("call 0".into(), None));
        let r1 = tokio::spawn(runner.clone()("call 1".into(), None));
        let r2 = tokio::spawn(runner.clone()("call 2".into(), None));

        // Give the spawned tasks time to enter the runner and bump depth.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let blocked = runner("call 3".into(), None).await;
        assert!(
            blocked.contains("max depth (3) exceeded"),
            "concurrent call at depth 3 should be rejected, got: {blocked}"
        );

        // Let the spawned tasks resolve (they'll timeout).
        let _ = tokio::join!(r0, r1, r2);
    }
}
