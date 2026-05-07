use crate::config::typed::ServiceConfig;
use crate::tracker::model::RetryEntry;
use std::time::{Duration, Instant};

pub fn schedule_retry(
    issue_id: String,
    identifier: String,
    attempt: u32,
    error: Option<String>,
    config: &ServiceConfig,
    is_continuation: bool,
) -> RetryEntry {
    let delay = if is_continuation {
        Duration::from_millis(1000)
    } else {
        let base = 10_000u64;
        let max_backoff = config.max_retry_backoff_ms();
        let delay = base.saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)));
        Duration::from_millis(delay.min(max_backoff))
    };
    RetryEntry {
        issue_id,
        identifier,
        attempt,
        due_at: Instant::now() + delay,
        error,
    }
}
