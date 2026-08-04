use std::env;

pub fn stream_idle_timeout() -> f64 {
    env::var("GREMLINS_STREAM_IDLE_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600.0)
}

pub const STREAM_IDLE_BACKOFF: [f64; 3] = [60.0, 300.0, 600.0];

pub fn validate_max_retries(max_retries: usize) -> Result<(), String> {
    if max_retries > STREAM_IDLE_BACKOFF.len() {
        Err(format!(
            "max_retries={max_retries} exceeds backoff schedule length {}",
            STREAM_IDLE_BACKOFF.len()
        ))
    } else {
        Ok(())
    }
}
