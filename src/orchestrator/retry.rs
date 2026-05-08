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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_config() -> ServiceConfig {
        ServiceConfig::new(
            serde_json::Map::<String, serde_json::Value>::new(),
            PathBuf::from("/tmp"),
            "".into(),
        )
    }

    #[test]
    fn test_schedule_retry_continuation() {
        let config = make_config();
        let before = Instant::now();
        let entry = schedule_retry("id1".into(), "ISSUE-1".into(), 1, None, &config, true);
        let after = Instant::now();
        assert_eq!(entry.issue_id, "id1");
        assert_eq!(entry.identifier, "ISSUE-1");
        assert_eq!(entry.attempt, 1);
        assert!(entry.error.is_none());
        assert!(entry.due_at >= before + Duration::from_millis(1000));
        assert!(entry.due_at <= after + Duration::from_millis(1000));
    }

    #[test]
    fn test_schedule_retry_failure_first_attempt() {
        let config = make_config();
        let before = Instant::now();
        let entry = schedule_retry(
            "id1".into(),
            "ISSUE-1".into(),
            1,
            Some("error".into()),
            &config,
            false,
        );
        let after = Instant::now();
        assert_eq!(entry.attempt, 1);
        assert_eq!(entry.error, Some("error".into()));
        assert!(entry.due_at >= before + Duration::from_millis(10000));
        assert!(entry.due_at <= after + Duration::from_millis(10000));
    }

    #[test]
    fn test_schedule_retry_failure_exponential() {
        let config = make_config();
        let before = Instant::now();
        let entry = schedule_retry(
            "id1".into(),
            "ISSUE-1".into(),
            3,
            Some("err".into()),
            &config,
            false,
        );
        let after = Instant::now();
        assert!(entry.due_at >= before + Duration::from_millis(40000));
        assert!(entry.due_at <= after + Duration::from_millis(40000));
    }

    #[test]
    fn test_schedule_retry_max_backoff() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut agent = serde_json::Map::<String, serde_json::Value>::new();
        agent.insert(
            "max_retry_backoff_ms".into(),
            serde_json::Value::Number(15000.into()),
        );
        raw.insert("agent".into(), serde_json::Value::Object(agent));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let before = Instant::now();
        let entry = schedule_retry(
            "id1".into(),
            "ISSUE-1".into(),
            10,
            Some("err".into()),
            &config,
            false,
        );
        let after = Instant::now();
        assert!(entry.due_at >= before + Duration::from_millis(15000));
        assert!(entry.due_at <= after + Duration::from_millis(15000));
    }
}
