use serde::{Deserialize, Serialize};

/// Aggregated token usage summary from an agent run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageStats {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub reasoning_tokens: u64,
    pub turns: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedRun {
    pub exit_code: i32,
    pub text_result: Option<String>,
    pub events: Option<Vec<serde_json::Value>>,
    pub cost_usd: Option<f64>,
    pub token_usage: Option<UsageStats>,
}
