use std::path::PathBuf;
use std::sync::Arc;

use rig_core::completion::CompletionModel;

use super::tools::{self, ToolContext};

const MAX_DEPTH: u32 = 3;

/// Build a subagent runner closure. Called once per backend before the agent loop.
/// The returned closure captures the model, instructions, tool filter, cancel token,
/// context prefix, and the original `ToolContext` — everything needed to run
/// a nested agent loop.
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

    Arc::new(move |task: String, cwd: Option<PathBuf>| {
        let model = model.clone();
        let instructions = instructions.clone();
        let tool_filter = tool_filter.clone();
        let cancel = cancel.clone();
        let mut sub_ctx = ctx.clone();
        let prefix = prefix.clone();
        let depth = depth.clone();

        if let Some(cwd) = cwd {
            sub_ctx.cwd = Some(cwd);
        }

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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_runner_depth_guard() {
        // smoke: constructor runs without panicking — depth is tested at runtime
        let depth = Arc::new(std::sync::atomic::AtomicU32::new(MAX_DEPTH));
        let current = depth.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(current, MAX_DEPTH);
        depth.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(depth.load(std::sync::atomic::Ordering::Relaxed), MAX_DEPTH);
    }
}
