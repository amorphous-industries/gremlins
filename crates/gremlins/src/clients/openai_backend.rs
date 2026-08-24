use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::future::join_all;
use futures::StreamExt;
use rig_core::client::CompletionClient;
use rig_core::completion::message::{AssistantContent, ToolCall};
use rig_core::completion::{CompletionModel, Message, ToolDefinition};
use rig_core::providers::openai;
use rig_core::streaming::StreamedAssistantContent;
use rig_core::OneOrMany;
use tokio::sync::Notify;

use super::backend::{Backend, ClientError, RunParams};
use super::config::{self, validate_max_retries, STREAM_IDLE_BACKOFF};
use super::protocol::CompletedRun;
use super::retry;
use super::stream;
use super::tools::{self, ToolContext};

const DEFAULT_TEMPERATURE: f64 = 0.4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiProvider {
    OpenAi,
    Xai,
    OpenRouter,
}

impl OpenAiProvider {
    pub fn api_key_env(self) -> &'static str {
        match self {
            Self::OpenAi => "OPENAI_API_KEY",
            Self::Xai => "XAI_API_KEY",
            Self::OpenRouter => "OPENROUTER_API_KEY",
        }
    }

    pub fn base_url(self) -> &'static str {
        match self {
            Self::OpenAi => "https://api.openai.com/v1",
            Self::Xai => "https://api.x.ai/v1",
            Self::OpenRouter => "https://openrouter.ai/api/v1",
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Self::OpenAi | Self::OpenRouter => "gpt-4o",
            Self::Xai => "grok-4",
        }
    }
}

pub(crate) struct CancelToken {
    flag: AtomicBool,
    notify: Notify,
}

impl CancelToken {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            flag: AtomicBool::new(false),
            notify: Notify::new(),
        })
    }

    fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
        self.notify.notify_waiters();
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    async fn cancelled(&self) {
        let notified = self.notify.notified();
        if self.flag.load(Ordering::Relaxed) {
            return;
        }
        notified.await;
    }
}

#[derive(Clone)]
struct RunContext {
    params: RunParams,
    prefix: String,
    idle_timeout: f64,
}

pub struct OpenAiBackend {
    provider: OpenAiProvider,
    client: openai::CompletionsClient,
    model: String,
    instructions: String,
    tool_filter: Option<Vec<String>>,
    last_ctx: Mutex<Option<RunContext>>,
    cancels: Mutex<HashMap<u64, Arc<CancelToken>>>,
    next_id: AtomicU64,
}

impl OpenAiBackend {
    pub fn new(
        provider: OpenAiProvider,
        client: openai::CompletionsClient,
        model: String,
        instructions: String,
        tool_filter: Option<Vec<String>>,
    ) -> Self {
        let model = if model.is_empty() {
            provider.default_model().to_string()
        } else {
            model
        };
        Self {
            provider,
            client,
            model,
            instructions,
            tool_filter,
            last_ctx: Mutex::new(None),
            cancels: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    fn extra_params(&self) -> Option<serde_json::Value> {
        extra_params(self.provider)
    }

    fn effective_model(&self, override_model: Option<&str>) -> String {
        match override_model {
            Some(m) if !m.is_empty() => m.to_string(),
            _ => self.model.clone(),
        }
    }

    async fn attempt(&self, prompt: &str, ctx: &RunContext) -> Result<CompletedRun, ClientError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let cancel = CancelToken::new();
        self.cancels.lock().unwrap().insert(id, cancel.clone());
        let result = self.attempt_inner(prompt, ctx, cancel).await;
        self.cancels.lock().unwrap().remove(&id);
        result
    }

    async fn attempt_inner(
        &self,
        prompt: &str,
        ctx: &RunContext,
        cancel: Arc<CancelToken>,
    ) -> Result<CompletedRun, ClientError> {
        let model_name = self.effective_model(ctx.params.model.as_deref());
        let model = self.client.completion_model(&model_name);
        let mut ctx = ctx.clone();
        ctx.params.model = Some(model_name);
        run_agent_loop(
            &model,
            prompt,
            &ctx,
            cancel,
            LoopOpts {
                instructions: &self.instructions,
                extra: self.extra_params(),
                tool_filter: self.tool_filter.as_deref(),
            },
        )
        .await
    }
}

struct LoopOpts<'a> {
    instructions: &'a str,
    extra: Option<serde_json::Value>,
    tool_filter: Option<&'a [String]>,
}

async fn run_agent_loop<M: CompletionModel + Clone + Send + Sync + 'static>(
    model: &M,
    prompt: &str,
    ctx: &RunContext,
    cancel: Arc<CancelToken>,
    opts: LoopOpts<'_>,
) -> Result<CompletedRun, ClientError> {
    let cwd = ctx.params.cwd.clone();
    let extra_env = ctx.params.extra_env.clone();
    let prefix = ctx.prefix.clone();
    let raw_path = ctx.params.raw_path.clone();
    let capture_events = ctx.params.capture_events;
    let idle_timeout = ctx.idle_timeout;
    let model_name = ctx
        .params
        .model
        .clone()
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "model".into());

    let cwd_display = cwd
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "?".into());
    stream::emit_init(&prefix, &model_name, &cwd_display);
    stream::flush();

    if cwd.is_none() {
        eprintln!("{prefix}warning: no cwd set for worktree enforcement");
    }

    let mut raw = raw_path
        .as_ref()
        .map(|p| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
        })
        .transpose()
        .map_err(|e| ClientError::Runtime {
            message: format!("failed to open raw_path: {e}"),
        })?;
    let mut captured: Option<Vec<serde_json::Value>> = if capture_events {
        Some(Vec::new())
    } else {
        None
    };

    let worktree = tools::worktree_root(cwd.as_deref());
    let audit_log = raw_path.as_ref().map(|p| tools::audit_log_path(p));
    let max_turns = config::openai_agents_max_turns();
    let mut tool_ctx = ToolContext {
        cwd: cwd.clone(),
        extra_env,
        worktree_root: worktree,
        audit_log,
        allowed_tools: opts.tool_filter.map(|s| s.to_vec()),
        subagent_fn: None,
    };
    let tool_defs = tools::tool_definitions(opts.tool_filter);

    // Wire up the subagent runner before entering the turn loop.
    let runner = super::subagent::make_runner(
        model.clone(),
        opts.instructions.to_string(),
        opts.tool_filter.map(|f| f.to_vec()),
        cancel.clone(),
        tool_ctx.clone(),
        prefix.clone(),
        idle_timeout,
        max_turns,
    );
    tool_ctx.subagent_fn = Some(runner);

    run_agent_loop_core(
        model,
        prompt,
        &tool_ctx,
        &tool_defs,
        &cancel,
        &opts,
        &prefix,
        max_turns,
        idle_timeout,
        &mut raw,
        &mut captured,
        false,
    )
    .await
}

/// Nested agent loop — same logic as the parent loop but without stream
/// emissions, raw file writes, or event capture. Used by the subagent tool.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_agent_loop_nested<M: CompletionModel + Clone + Send + Sync + 'static>(
    model: &M,
    prompt: &str,
    tool_ctx: &ToolContext,
    cancel: &CancelToken,
    instructions: &str,
    tool_filter: Option<&[String]>,
    prefix: &str,
    idle_timeout: f64,
    max_turns: usize,
) -> Result<CompletedRun, ClientError> {
    let opts = LoopOpts {
        instructions,
        extra: None,
        tool_filter,
    };
    let tool_defs = tools::tool_definitions(tool_filter);
    let mut raw: Option<std::fs::File> = None;
    let mut captured: Option<Vec<serde_json::Value>> = None;
    run_agent_loop_core(
        model,
        prompt,
        tool_ctx,
        &tool_defs,
        cancel,
        &opts,
        prefix,
        max_turns,
        idle_timeout,
        &mut raw,
        &mut captured,
        true,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_agent_loop_core<M: CompletionModel>(
    model: &M,
    prompt: &str,
    tool_ctx: &ToolContext,
    tool_defs: &[ToolDefinition],
    cancel: &CancelToken,
    opts: &LoopOpts<'_>,
    prefix: &str,
    max_turns: usize,
    idle_timeout: f64,
    raw: &mut Option<std::fs::File>,
    captured: &mut Option<Vec<serde_json::Value>>,
    nested: bool,
) -> Result<CompletedRun, ClientError> {
    let mut history: Vec<Message> = Vec::new();
    let mut next_prompt = Message::user(prompt.to_string());
    let mut turns: usize = 0;
    let mut final_text = String::new();
    let mut timed_out = false;
    let mut stream_error: Option<String> = None;

    for _ in 0..max_turns {
        if cancel.is_cancelled() {
            return Err(ClientError::Runtime {
                message: "cancelled".into(),
            });
        }

        let mut builder = model
            .completion_request(next_prompt.clone())
            .preamble(opts.instructions.to_string())
            .messages(history.clone())
            .tools(tool_defs.to_vec())
            .temperature(DEFAULT_TEMPERATURE);
        if let Some(params) = opts.extra.clone() {
            builder = builder.additional_params(params);
        }

        let mut response = match builder.stream().await {
            Ok(s) => s,
            Err(e) => {
                stream_error = Some(e.to_string());
                break;
            }
        };

        let mut text = String::new();
        let mut reasoning = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut ended = false;

        loop {
            let item = tokio::select! {
                _ = cancel.cancelled() => {
                    response.cancel();
                    return Err(ClientError::Runtime {
                        message: "cancelled".into(),
                    });
                }
                timed = tokio::time::timeout(
                    Duration::from_secs_f64(idle_timeout),
                    response.next(),
                ) => timed,
            };
            match item {
                Err(_) => {
                    timed_out = true;
                    response.cancel();
                    break;
                }
                Ok(None) => {
                    ended = true;
                    break;
                }
                Ok(Some(Err(e))) => {
                    stream_error = Some(e.to_string());
                    break;
                }
                Ok(Some(Ok(chunk))) => {
                    apply_chunk(chunk, &mut text, &mut reasoning, &mut tool_calls);
                }
            }
        }

        if timed_out || stream_error.is_some() {
            break;
        }
        if !ended && tool_calls.is_empty() && text.is_empty() {
            break;
        }

        if !nested {
            if !reasoning.is_empty() {
                stream::emit_think(prefix, &reasoning);
            }
            if !text.is_empty() {
                stream::emit_text(prefix, &text);
                write_raw(raw, &assistant_text_event(&text));
                if let Some(evts) = captured.as_mut() {
                    evts.push(assistant_text_event(&text));
                }
                final_text = text.clone();
            }
        } else {
            final_text = text.clone();
        }

        if tool_calls.is_empty() {
            if !nested {
                stream::flush();
                emit_final(prefix, turns, "");
            }
            return Ok(CompletedRun {
                exit_code: 0,
                text_result: Some(final_text),
                events: captured.clone(),
                cost_usd: None,
            });
        }

        history.push(next_prompt);
        history.push(assistant_tool_message(&text, &tool_calls));

        // Phase 1: emit tool-start events, collect owned data for concurrent execution
        struct Job {
            id: String,
            call_id: Option<String>,
            name: String,
            args: String,
        }
        let mut jobs: Vec<Job> = Vec::new();
        for tc in &tool_calls {
            let args_json =
                serde_json::to_string(&tc.function.arguments).unwrap_or_else(|_| "{}".into());
            if !nested {
                stream::emit_tool(prefix, &tc.function.name, &key_arg(&tc.function.arguments));
                let tool_evt = tool_use_event(&tc.id, &tc.function.name, &tc.function.arguments);
                write_raw(raw, &tool_evt);
                if let Some(evts) = captured.as_mut() {
                    evts.push(tool_evt);
                }
            }
            jobs.push(Job {
                id: tc.id.clone(),
                call_id: tc.call_id.clone(),
                name: tc.function.name.clone(),
                args: args_json,
            });
        }

        // Phase 2: concurrent execution
        let ctx = tool_ctx.clone();
        let results = join_all(jobs.iter().map(|j| tools::invoke(&j.name, &ctx, &j.args))).await;

        // Phase 3: emit results in order
        let mut result_msgs = Vec::new();
        for (job, output) in jobs.into_iter().zip(results) {
            if !nested {
                stream::emit_result(prefix, &output, false);
                let result_evt = tool_result_event(&job.id, &output);
                write_raw(raw, &result_evt);
                if let Some(evts) = captured.as_mut() {
                    evts.push(result_evt);
                }
            }
            result_msgs.push(Message::tool_result_with_call_id(
                job.id,
                job.call_id,
                output,
            ));
        }
        turns += tool_calls.len();
        if !nested {
            stream::flush();
        }
        next_prompt = result_msgs.pop().unwrap_or_else(|| Message::user(""));
        history.extend(result_msgs);
    }

    if !nested {
        let suffix = if timed_out {
            " (timeout)"
        } else if stream_error.is_some() {
            " (stream-error)"
        } else {
            ""
        };
        emit_final(prefix, turns, suffix);
    }

    if timed_out {
        return Err(ClientError::Timeout {
            message: "openai stream idle timeout".into(),
        });
    }
    if let Some(msg) = stream_error {
        return Err(map_stream_error(msg));
    }
    Err(ClientError::Runtime {
        message: format!("exceeded max turns ({max_turns})"),
    })
}

fn extra_params(provider: OpenAiProvider) -> Option<serde_json::Value> {
    match provider {
        OpenAiProvider::Xai => Some(serde_json::json!({
            "reasoning": {"effort": "high", "summary": "auto"}
        })),
        OpenAiProvider::OpenRouter => Some(serde_json::json!({
            "parallel_tool_calls": true,
            "reasoning": {"effort": "high", "summary": "auto"}
        })),
        OpenAiProvider::OpenAi => Some(serde_json::json!({
            "parallel_tool_calls": true
        })),
    }
}

fn apply_chunk<R>(
    chunk: StreamedAssistantContent<R>,
    text: &mut String,
    reasoning: &mut String,
    tool_calls: &mut Vec<ToolCall>,
) {
    match chunk {
        StreamedAssistantContent::Text(t) => text.push_str(&t.text),
        StreamedAssistantContent::ToolCall { tool_call, .. } => tool_calls.push(tool_call),
        StreamedAssistantContent::ToolCallDelta { .. } => {}
        StreamedAssistantContent::Reasoning(r) => reasoning.push_str(&r.display_text()),
        StreamedAssistantContent::ReasoningDelta { reasoning: r, .. } => reasoning.push_str(&r),
        StreamedAssistantContent::Final(_) | StreamedAssistantContent::Unknown(_) => {}
    }
}

fn assistant_tool_message(text: &str, tool_calls: &[ToolCall]) -> Message {
    let mut contents = Vec::new();
    if !text.is_empty() {
        contents.push(AssistantContent::text(text.to_string()));
    }
    for tc in tool_calls {
        contents.push(AssistantContent::ToolCall(tc.clone()));
    }
    Message::Assistant {
        id: None,
        content: OneOrMany::from_iter_optional(contents)
            .unwrap_or_else(|| OneOrMany::one(AssistantContent::text(""))),
    }
}

fn key_arg(args: &serde_json::Value) -> String {
    if let Some(obj) = args.as_object() {
        for k in ["file_path", "command", "pattern", "url", "output_file"] {
            if let Some(v) = obj.get(k).and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    return v.to_string();
                }
            }
        }
    }
    String::new()
}

fn assistant_text_event(text: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "assistant",
        "message": {"content": [{"type": "text", "text": text}]}
    })
}

fn tool_use_event(id: &str, name: &str, input: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "type": "assistant",
        "message": {
            "content": [{
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input
            }]
        }
    })
}

fn tool_result_event(id: &str, content: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "user",
        "message": {
            "content": [{
                "type": "tool_result",
                "tool_use_id": id,
                "content": content
            }]
        }
    })
}

fn write_raw(raw: &mut Option<std::fs::File>, evt: &serde_json::Value) {
    if let Some(f) = raw {
        if let Ok(line) = serde_json::to_string(evt) {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
    }
}

fn emit_final(prefix: &str, turns: usize, suffix: &str) {
    eprintln!(
        "{} {}final: turns={turns} cost=not-reported{suffix}",
        stream::ts_internal(),
        prefix
    );
    stream::flush();
}

fn map_stream_error(msg: String) -> ClientError {
    if config::is_transient_stream_error(&msg) {
        ClientError::ApiServerError { message: msg }
    } else {
        ClientError::Runtime { message: msg }
    }
}

fn classify_retryable(e: &ClientError) -> bool {
    matches!(
        e,
        ClientError::Timeout { .. } | ClientError::ApiServerError { .. }
    )
}

fn retry_prompt(err: &ClientError, prompt: &str, on_timeout_prompt: Option<&str>) -> String {
    match err {
        ClientError::Timeout { .. } => on_timeout_prompt.unwrap_or(prompt).to_string(),
        _ => prompt.to_string(),
    }
}

#[async_trait]
impl Backend for OpenAiBackend {
    async fn run(&self, params: RunParams) -> Result<CompletedRun, ClientError> {
        validate_max_retries(params.max_retries)
            .map_err(|m| ClientError::Runtime { message: m })?;

        let idle_timeout = params
            .idle_timeout
            .unwrap_or_else(config::stream_idle_timeout);
        let prefix = if params.label.is_empty() {
            String::new()
        } else {
            format!("[{}] ", params.label)
        };
        let ctx = RunContext {
            params: params.clone(),
            prefix: prefix.clone(),
            idle_timeout,
        };
        *self.last_ctx.lock().unwrap() = Some(ctx.clone());

        let prompt = Mutex::new(params.prompt.clone());
        let timeout_prompt = params.on_timeout_prompt.clone();
        let backoff = &STREAM_IDLE_BACKOFF[..params.max_retries];

        retry::with_retry(
            backoff,
            classify_retryable,
            |attempt, e, wait| {
                let next = retry_prompt(e, &prompt.lock().unwrap(), timeout_prompt.as_deref());
                *prompt.lock().unwrap() = next;
                let cause = match e {
                    ClientError::Timeout { .. } => "stream idle timeout",
                    ClientError::ApiServerError { .. } => "transient-error",
                    _ => "error",
                };
                eprintln!(
                    "{} {}stream {cause}, retrying in {wait}s ({}/{})...",
                    stream::ts_internal(),
                    prefix,
                    attempt + 1,
                    params.max_retries
                );
            },
            || {
                let p = prompt.lock().unwrap().clone();
                let ctx = ctx.clone();
                async move { self.attempt(&p, &ctx).await }
            },
        )
        .await
    }

    async fn resume(&self) -> Result<CompletedRun, ClientError> {
        let params = {
            let guard = self.last_ctx.lock().unwrap();
            let ctx = guard.as_ref().ok_or_else(|| ClientError::Runtime {
                message: "resume() called before run()".into(),
            })?;
            ctx.params.clone()
        };
        self.run(params).await
    }

    fn reap_all(&self) {
        if let Ok(guard) = self.cancels.lock() {
            for token in guard.values() {
                token.cancel();
            }
        }
    }

    fn total_cost_usd(&self) -> Option<f64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    #[test]
    fn provider_identity() {
        assert_eq!(OpenAiProvider::OpenAi.api_key_env(), "OPENAI_API_KEY");
        assert_eq!(OpenAiProvider::Xai.api_key_env(), "XAI_API_KEY");
        assert_eq!(
            OpenAiProvider::OpenRouter.api_key_env(),
            "OPENROUTER_API_KEY"
        );
        assert_eq!(OpenAiProvider::Xai.base_url(), "https://api.x.ai/v1");
        assert_eq!(
            OpenAiProvider::OpenRouter.base_url(),
            "https://openrouter.ai/api/v1"
        );
        assert_eq!(OpenAiProvider::Xai.default_model(), "grok-4");
    }

    #[test]
    fn xai_reasoning_params() {
        let params = extra_params(OpenAiProvider::Xai).unwrap();
        assert_eq!(params["reasoning"]["effort"], "high");
        assert_eq!(params["reasoning"]["summary"], "auto");
    }

    #[test]
    fn event_shapes() {
        let text = assistant_text_event("hello");
        assert_eq!(text["type"], "assistant");
        assert_eq!(text["message"]["content"][0]["text"], "hello");

        let tool = tool_use_event("id42", "Bash", &serde_json::json!({"command": "ls"}));
        assert_eq!(tool["message"]["content"][0]["type"], "tool_use");
        assert_eq!(tool["message"]["content"][0]["name"], "Bash");
        assert_eq!(tool["message"]["content"][0]["id"], "id42");

        let result = tool_result_event("id42", "ok");
        assert_eq!(result["type"], "user");
        assert_eq!(result["message"]["content"][0]["tool_use_id"], "id42");
        assert_eq!(result["message"]["content"][0]["content"], "ok");
    }

    #[test]
    fn key_arg_picks_known_fields() {
        assert_eq!(
            key_arg(&serde_json::json!({"file_path": "/tmp/x.py"})),
            "/tmp/x.py"
        );
        assert_eq!(
            key_arg(&serde_json::json!({"command": "echo hi"})),
            "echo hi"
        );
        assert_eq!(key_arg(&serde_json::json!({})), "");
    }

    #[test]
    fn transient_classifier() {
        assert!(config::is_transient_stream_error(
            "The model is currently at capacity"
        ));
        assert!(config::is_transient_stream_error("rate limit exceeded"));
        assert!(!config::is_transient_stream_error("Invalid API key"));
        assert!(matches!(
            map_stream_error("rate limit exceeded".into()),
            ClientError::ApiServerError { .. }
        ));
        assert!(matches!(
            map_stream_error("Invalid API key".into()),
            ClientError::Runtime { .. }
        ));
    }

    #[tokio::test]
    async fn idle_timeout_on_pending_stream() {
        let mut pending = stream::pending::<()>();
        let timed = tokio::time::timeout(Duration::from_secs_f64(0.05), pending.next()).await;
        assert!(timed.is_err());
    }

    #[tokio::test]
    async fn cancel_token_wakes_waiters() {
        let token = CancelToken::new();
        let t = token.clone();
        let handle = tokio::spawn(async move {
            t.cancelled().await;
        });
        token.cancel();
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .unwrap()
            .unwrap();
        assert!(token.is_cancelled());
    }

    fn loop_opts(filter: Option<&[String]>) -> LoopOpts<'_> {
        LoopOpts {
            instructions: "",
            extra: None,
            tool_filter: filter,
        }
    }

    fn test_ctx(
        cwd: Option<std::path::PathBuf>,
        raw_path: Option<std::path::PathBuf>,
    ) -> RunContext {
        RunContext {
            params: RunParams {
                prompt: "hi".into(),
                label: "t".into(),
                model: Some("mock".into()),
                raw_path,
                capture_events: true,
                on_timeout_prompt: None,
                max_retries: 0,
                cwd,
                idle_timeout: Some(0.05),
                extra_env: None,
            },
            prefix: "[t] ".into(),
            idle_timeout: 0.05,
        }
    }

    #[derive(Clone)]
    struct PendingModel;

    impl CompletionModel for PendingModel {
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
    async fn loop_idle_timeout_is_client_timeout() {
        let ctx = test_ctx(None, None);
        let cancel = CancelToken::new();
        let err = run_agent_loop(&PendingModel, "hi", &ctx, cancel, loop_opts(None))
            .await
            .unwrap_err();
        assert!(matches!(err, ClientError::Timeout { .. }));
    }

    #[tokio::test]
    async fn loop_tool_then_text() {
        let dir = std::env::temp_dir().join(format!(
            "gremlins-oa-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("out.txt");
        let model = rig_core::test_utils::MockCompletionModel::from_stream_turns([
            vec![
                rig_core::test_utils::MockStreamEvent::tool_call(
                    "c1",
                    "Write",
                    serde_json::json!({
                        "file_path": target.to_str().unwrap(),
                        "content": "hello"
                    }),
                ),
                rig_core::test_utils::MockStreamEvent::final_response_with_default_usage(),
            ],
            vec![
                rig_core::test_utils::MockStreamEvent::text("wrote it"),
                rig_core::test_utils::MockStreamEvent::final_response_with_default_usage(),
            ],
        ]);
        let mut ctx = test_ctx(Some(dir.clone()), None);
        ctx.idle_timeout = 5.0;
        ctx.params.idle_timeout = Some(5.0);
        let cancel = CancelToken::new();
        let result = run_agent_loop(&model, "write", &ctx, cancel, loop_opts(None))
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.text_result.as_deref(), Some("wrote it"));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "hello");
        let events = result.events.unwrap();
        assert!(events
            .iter()
            .any(|e| e["message"]["content"][0]["type"] == "tool_use"));
        assert!(events
            .iter()
            .any(|e| e["message"]["content"][0]["type"] == "tool_result"));
    }

    #[tokio::test]
    async fn loop_bash_tool_uses_cwd() {
        // Reproduction: a Bash tool call issued by the model must run inside
        // the gremlin worktree (cwd), not the process's own current directory.
        let dir = std::env::temp_dir().join(format!(
            "gremlins-oa-cwd-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("marker.txt"), "in-worktree").unwrap();

        // Use a relative Bash command so any cwd mishandling shows up: `pwd`
        // must resolve to `dir`, not the process cwd.
        let model = rig_core::test_utils::MockCompletionModel::from_stream_turns([
            vec![
                rig_core::test_utils::MockStreamEvent::tool_call(
                    "c1",
                    "Bash",
                    serde_json::json!({ "command": "pwd; ls" }),
                ),
                rig_core::test_utils::MockStreamEvent::final_response_with_default_usage(),
            ],
            vec![
                rig_core::test_utils::MockStreamEvent::text("saw it"),
                rig_core::test_utils::MockStreamEvent::final_response_with_default_usage(),
            ],
        ]);
        let mut ctx = test_ctx(Some(dir.clone()), None);
        ctx.idle_timeout = 5.0;
        ctx.params.idle_timeout = Some(5.0);
        let cancel = CancelToken::new();
        let result = run_agent_loop(&model, "where am i", &ctx, cancel, loop_opts(None))
            .await
            .unwrap();
        let events = result.events.unwrap();
        let result_evt = events
            .iter()
            .find(|e| e["message"]["content"][0]["type"] == "tool_result")
            .unwrap();
        let content = result_evt["message"]["content"][0]["content"]
            .as_str()
            .unwrap();
        assert!(
            content.contains(dir.to_str().unwrap()),
            "Bash tool should run in cwd={}, got: {content}",
            dir.display()
        );
        assert!(
            content.contains("marker.txt"),
            "Bash `ls` should see worktree marker, got: {content}"
        );
    }

    #[tokio::test]
    async fn loop_filtered_tool_refused() {
        let dir = std::env::temp_dir().join(format!(
            "gremlins-oa-filter-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("must-not-exist.txt");
        let model = rig_core::test_utils::MockCompletionModel::from_stream_turns([
            vec![
                rig_core::test_utils::MockStreamEvent::tool_call(
                    "c1",
                    "Write",
                    serde_json::json!({
                        "file_path": target.to_str().unwrap(),
                        "content": "nope"
                    }),
                ),
                rig_core::test_utils::MockStreamEvent::final_response_with_default_usage(),
            ],
            vec![
                rig_core::test_utils::MockStreamEvent::text("blocked"),
                rig_core::test_utils::MockStreamEvent::final_response_with_default_usage(),
            ],
        ]);
        let mut ctx = test_ctx(Some(dir.clone()), None);
        ctx.idle_timeout = 5.0;
        ctx.params.idle_timeout = Some(5.0);
        let cancel = CancelToken::new();
        let filter = vec!["Read".to_string()];
        let result = run_agent_loop(&model, "write", &ctx, cancel, loop_opts(Some(&filter)))
            .await
            .unwrap();
        assert!(!target.exists());
        let events = result.events.unwrap();
        let result_evt = events
            .iter()
            .find(|e| e["message"]["content"][0]["type"] == "tool_result")
            .unwrap();
        let content = result_evt["message"]["content"][0]["content"]
            .as_str()
            .unwrap();
        assert!(content.contains("unknown tool"));
    }

    #[test]
    fn retry_prompt_swaps_only_on_timeout() {
        let timeout = ClientError::Timeout {
            message: "idle".into(),
        };
        let api = ClientError::ApiServerError {
            message: "rate limit".into(),
        };
        assert_eq!(
            retry_prompt(&timeout, "orig", Some("timeout-prompt")),
            "timeout-prompt"
        );
        assert_eq!(retry_prompt(&api, "orig", Some("timeout-prompt")), "orig");
        assert_eq!(retry_prompt(&timeout, "orig", None), "orig");
    }

    #[test]
    fn reap_all_cancels_every_token() {
        let client = openai::Client::builder()
            .api_key(rig_core::client::BearerAuth::from("sk-test"))
            .base_url("https://api.openai.com/v1")
            .build()
            .unwrap()
            .completions_api();
        let backend = OpenAiBackend::new(
            OpenAiProvider::OpenAi,
            client,
            "gpt-4o".into(),
            String::new(),
            None,
        );
        let a = CancelToken::new();
        let b = CancelToken::new();
        backend.cancels.lock().unwrap().insert(1, a.clone());
        backend.cancels.lock().unwrap().insert(2, b.clone());
        backend.reap_all();
        assert!(a.is_cancelled());
        assert!(b.is_cancelled());
    }

    #[tokio::test]
    async fn loop_writes_audit_jsonl() {
        let dir = std::env::temp_dir().join(format!(
            "gremlins-oa-audit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let raw = dir.join("run.jsonl");
        let target = dir.join("out.txt");
        let model = rig_core::test_utils::MockCompletionModel::from_stream_turns([
            vec![
                rig_core::test_utils::MockStreamEvent::tool_call(
                    "c1",
                    "Write",
                    serde_json::json!({
                        "file_path": target.to_str().unwrap(),
                        "content": "x"
                    }),
                ),
                rig_core::test_utils::MockStreamEvent::final_response_with_default_usage(),
            ],
            vec![
                rig_core::test_utils::MockStreamEvent::text("ok"),
                rig_core::test_utils::MockStreamEvent::final_response_with_default_usage(),
            ],
        ]);
        let mut ctx = test_ctx(Some(dir.clone()), Some(raw.clone()));
        ctx.idle_timeout = 5.0;
        ctx.params.idle_timeout = Some(5.0);
        let cancel = CancelToken::new();
        run_agent_loop(&model, "write", &ctx, cancel, loop_opts(None))
            .await
            .unwrap();
        let audit = dir.join("run.audit.jsonl");
        assert!(audit.exists());
        let entry: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&audit).unwrap()).unwrap();
        assert_eq!(entry["tool"], "Write");
        assert_eq!(entry["status"], "ok");
    }

    #[tokio::test]
    async fn loop_concurrent_multi_tool_in_one_turn() {
        let dir = std::env::temp_dir().join(format!(
            "gremlins-oa-conc-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let f1 = dir.join("a.txt");
        let f2 = dir.join("b.txt");
        std::fs::write(&f1, "alpha").unwrap();
        std::fs::write(&f2, "beta").unwrap();

        let model = rig_core::test_utils::MockCompletionModel::from_stream_turns([
            vec![
                rig_core::test_utils::MockStreamEvent::tool_call(
                    "c1",
                    "Read",
                    serde_json::json!({"file_path": f1.to_str().unwrap()}),
                ),
                rig_core::test_utils::MockStreamEvent::tool_call(
                    "c2",
                    "Read",
                    serde_json::json!({"file_path": f2.to_str().unwrap()}),
                ),
                rig_core::test_utils::MockStreamEvent::final_response_with_default_usage(),
            ],
            vec![
                rig_core::test_utils::MockStreamEvent::text("both read"),
                rig_core::test_utils::MockStreamEvent::final_response_with_default_usage(),
            ],
        ]);
        let mut ctx = test_ctx(Some(dir.clone()), None);
        ctx.idle_timeout = 5.0;
        ctx.params.idle_timeout = Some(5.0);
        let cancel = CancelToken::new();
        let result = run_agent_loop(&model, "read both", &ctx, cancel, loop_opts(None))
            .await
            .unwrap();
        assert_eq!(result.text_result.as_deref(), Some("both read"));
        let events = result.events.unwrap();
        // Two tool_use events, then two tool_result events, in order
        let tool_uses: Vec<_> = events
            .iter()
            .filter(|e| e["message"]["content"][0]["type"] == "tool_use")
            .collect();
        assert_eq!(tool_uses.len(), 2);
        assert_eq!(tool_uses[0]["message"]["content"][0]["id"], "c1");
        assert_eq!(tool_uses[1]["message"]["content"][0]["id"], "c2");
        let results: Vec<_> = events
            .iter()
            .filter(|e| e["message"]["content"][0]["type"] == "tool_result")
            .collect();
        assert_eq!(results.len(), 2);
        assert!(results[0]["message"]["content"][0]["content"]
            .as_str()
            .unwrap()
            .contains("alpha"));
        assert!(results[1]["message"]["content"][0]["content"]
            .as_str()
            .unwrap()
            .contains("beta"));
    }

    #[tokio::test]
    async fn loop_concurrent_mixed_tools_with_failure() {
        let dir = std::env::temp_dir().join(format!(
            "gremlins-oa-mixed-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let read_file = dir.join("readme.txt");
        let write_file = dir.join("out.txt");
        std::fs::write(&read_file, "before").unwrap();

        let model = rig_core::test_utils::MockCompletionModel::from_stream_turns([
            vec![
                rig_core::test_utils::MockStreamEvent::tool_call(
                    "r1",
                    "Read",
                    serde_json::json!({"file_path": read_file.to_str().unwrap()}),
                ),
                rig_core::test_utils::MockStreamEvent::tool_call(
                    "w1",
                    "Write",
                    serde_json::json!({
                        "file_path": write_file.to_str().unwrap(),
                        "content": "mixed"
                    }),
                ),
                rig_core::test_utils::MockStreamEvent::tool_call(
                    "b1",
                    "Bash",
                    serde_json::json!({"command": "exit 1"}),
                ),
                rig_core::test_utils::MockStreamEvent::final_response_with_default_usage(),
            ],
            vec![
                rig_core::test_utils::MockStreamEvent::text("done"),
                rig_core::test_utils::MockStreamEvent::final_response_with_default_usage(),
            ],
        ]);
        let mut ctx = test_ctx(Some(dir.clone()), None);
        ctx.idle_timeout = 5.0;
        ctx.params.idle_timeout = Some(5.0);
        let cancel = CancelToken::new();
        let result = run_agent_loop(&model, "mix", &ctx, cancel, loop_opts(None))
            .await
            .unwrap();
        assert_eq!(result.text_result.as_deref(), Some("done"));
        let events = result.events.unwrap();
        let tool_uses: Vec<_> = events
            .iter()
            .filter(|e| e["message"]["content"][0]["type"] == "tool_use")
            .collect();
        assert_eq!(tool_uses.len(), 3);
        assert_eq!(tool_uses[0]["message"]["content"][0]["name"], "Read");
        assert_eq!(tool_uses[1]["message"]["content"][0]["name"], "Write");
        assert_eq!(tool_uses[2]["message"]["content"][0]["name"], "Bash");
        let results: Vec<_> = events
            .iter()
            .filter(|e| e["message"]["content"][0]["type"] == "tool_result")
            .collect();
        assert_eq!(results.len(), 3);
        // Read result contains "before"
        assert!(results[0]["message"]["content"][0]["content"]
            .as_str()
            .unwrap()
            .contains("before"));
        // Write result confirms write
        assert_eq!(
            results[1]["message"]["content"][0]["content"]
                .as_str()
                .unwrap(),
            "OK"
        );
        // Bash result contains exit failure info
        let bash_content = results[2]["message"]["content"][0]["content"]
            .as_str()
            .unwrap();
        assert!(
            bash_content.contains("[exit 1]"),
            "bash failure not as expected: {bash_content}"
        );
        // All three tool_results map to their ids
        assert_eq!(results[0]["message"]["content"][0]["tool_use_id"], "r1");
        assert_eq!(results[1]["message"]["content"][0]["tool_use_id"], "w1");
        assert_eq!(results[2]["message"]["content"][0]["tool_use_id"], "b1");
        // Write actually happened
        assert_eq!(std::fs::read_to_string(&write_file).unwrap(), "mixed");
    }
}
