use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use sympheo::config::typed::ServiceConfig;
use sympheo::error::SympheoError;
use sympheo::orchestrator::tick::Orchestrator;
use sympheo::tracker::model::Issue;
use sympheo::tracker::IssueTracker;

struct MockTracker {
    candidates: Vec<Issue>,
    by_states: Vec<Issue>,
    by_ids: Vec<Issue>,
}

#[async_trait]
impl IssueTracker for MockTracker {
    async fn fetch_candidate_issues(&self) -> Result<Vec<Issue>, SympheoError> {
        Ok(self.candidates.clone())
    }

    async fn fetch_issues_by_states(
        &self,
        _states: &[String],
    ) -> Result<Vec<Issue>, SympheoError> {
        Ok(self.by_states.clone())
    }

    async fn fetch_issue_states_by_ids(
        &self,
        _ids: &[String],
    ) -> Result<Vec<Issue>, SympheoError> {
        Ok(self.by_ids.clone())
    }
}

fn valid_config() -> ServiceConfig {
    let mut raw = serde_yaml::Mapping::new();
    let mut tracker = serde_yaml::Mapping::new();
    tracker.insert(
        serde_yaml::Value::String("kind".into()),
        serde_yaml::Value::String("github".into()),
    );
    tracker.insert(
        serde_yaml::Value::String("api_key".into()),
        serde_yaml::Value::String("key".into()),
    );
    tracker.insert(
        serde_yaml::Value::String("project_slug".into()),
        serde_yaml::Value::String("owner/repo".into()),
    );
    tracker.insert(
        serde_yaml::Value::String("project_number".into()),
        serde_yaml::Value::Number(1.into()),
    );
    raw.insert(
        serde_yaml::Value::String("tracker".into()),
        serde_yaml::Value::Mapping(tracker),
    );
    ServiceConfig::new(raw, PathBuf::from("/tmp"), "prompt".into())
}

fn make_issue(id: &str, identifier: &str, state: &str) -> Issue {
    Issue {
        id: id.into(),
        identifier: identifier.into(),
        title: "test".into(),
        description: None,
        priority: None,
        state: state.into(),
        branch_name: None,
        url: None,
        labels: vec![],
        blocked_by: vec![],
        ..Default::default()
    }
}

#[tokio::test]
async fn test_orchestrator_tick_no_candidates() {
    let config = valid_config();
    let tracker = Arc::new(MockTracker {
        candidates: vec![],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, std::collections::HashMap::new(), None).unwrap();
    orch.tick().await;

    let state = orch.state.read().await;
    assert!(state.running.is_empty());
    assert!(state.retry_attempts.is_empty());
}

#[tokio::test]
async fn test_orchestrator_tick_dispatches_eligible_issue() {
    let config = valid_config();
    let issue = make_issue("1", "TEST-1", "todo");
    let tracker = Arc::new(MockTracker {
        candidates: vec![issue.clone()],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, std::collections::HashMap::new(), None).unwrap();
    orch.tick().await;

    let state = orch.state.read().await;
    assert!(state.claimed.contains("1"));
    assert!(state.running.contains_key("1"));
}

#[tokio::test]
async fn test_orchestrator_tick_skips_blocked_todo() {
    let config = valid_config();
    let issue = Issue {
        blocked_by: vec![sympheo::tracker::model::BlockerRef {
            id: Some("2".into()),
            identifier: Some("TEST-2".into()),
            state: Some("in progress".into()),
        }],
        ..make_issue("1", "TEST-1", "todo")
    };
    let tracker = Arc::new(MockTracker {
        candidates: vec![issue],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, std::collections::HashMap::new(), None).unwrap();
    orch.tick().await;

    let state = orch.state.read().await;
    assert!(!state.claimed.contains("1"));
    assert!(!state.running.contains_key("1"));
}

#[tokio::test]
async fn test_orchestrator_tick_skips_terminal_issue() {
    let config = valid_config();
    let issue = make_issue("1", "TEST-1", "closed");
    let tracker = Arc::new(MockTracker {
        candidates: vec![issue],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, std::collections::HashMap::new(), None).unwrap();
    orch.tick().await;

    let state = orch.state.read().await;
    assert!(!state.claimed.contains("1"));
}

#[tokio::test]
async fn test_orchestrator_tick_respects_concurrency_limit() {
    let mut config = valid_config();
    config = ServiceConfig::new(config.raw().clone(), PathBuf::from("/tmp"), "prompt".into());
    let mut raw = config.raw().clone();
    let mut agent = serde_yaml::Mapping::new();
    agent.insert(
        serde_yaml::Value::String("max_concurrent_agents".into()),
        serde_yaml::Value::Number(1.into()),
    );
    raw.insert(
        serde_yaml::Value::String("agent".into()),
        serde_yaml::Value::Mapping(agent),
    );
    let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "prompt".into());

    let tracker = Arc::new(MockTracker {
        candidates: vec![
            make_issue("1", "TEST-1", "todo"),
            make_issue("2", "TEST-2", "todo"),
        ],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, std::collections::HashMap::new(), None).unwrap();
    orch.tick().await;

    let state = orch.state.read().await;
    assert_eq!(state.running.len(), 1);
}

#[tokio::test]
async fn test_orchestrator_reload_config() {
    let config = valid_config();
    let tracker = Arc::new(MockTracker {
        candidates: vec![],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, std::collections::HashMap::new(), None).unwrap();

    let mut new_raw = serde_yaml::Mapping::new();
    let mut polling = serde_yaml::Mapping::new();
    polling.insert(
        serde_yaml::Value::String("interval_ms".into()),
        serde_yaml::Value::Number(10000.into()),
    );
    new_raw.insert(
        serde_yaml::Value::String("polling".into()),
        serde_yaml::Value::Mapping(polling),
    );
    let mut agent = serde_yaml::Mapping::new();
    agent.insert(
        serde_yaml::Value::String("max_concurrent_agents".into()),
        serde_yaml::Value::Number(5.into()),
    );
    new_raw.insert(
        serde_yaml::Value::String("agent".into()),
        serde_yaml::Value::Mapping(agent),
    );
    let new_config = ServiceConfig::new(new_raw, PathBuf::from("/tmp"), "prompt".into());

    orch.reload_config(new_config, std::collections::HashMap::new()).await;

    let state = orch.state.read().await;
    assert_eq!(state.poll_interval_ms, 10000);
    assert_eq!(state.max_concurrent_agents, 5);
}

#[tokio::test]
async fn test_orchestrator_handle_worker_exit_normal() {
    let config = valid_config();
    let tracker = Arc::new(MockTracker {
        candidates: vec![],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, std::collections::HashMap::new(), None).unwrap();

    {
        let mut state = orch.state.write().await;
        state.running.insert(
            "1".into(),
            sympheo::orchestrator::state::RunningEntry {
                issue: make_issue("1", "TEST-1", "todo"),
                session: None,
                started_at: chrono::Utc::now(),
                retry_attempt: None,
                turn_count: 0,
                cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                stagnation_counter: 0,
                last_state_change_at: chrono::Utc::now(),
            },
        );
        state.claimed.insert("1".into());
    }

    orch.handle_worker_exit("1", true, None).await;

    let state = orch.state.read().await;
    assert!(!state.running.contains_key("1"));
    assert!(state.completed.contains("1"));
    assert!(state.retry_attempts.contains_key("1"));
}

#[tokio::test]
async fn test_orchestrator_handle_worker_exit_error() {
    let config = valid_config();
    let tracker = Arc::new(MockTracker {
        candidates: vec![],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, std::collections::HashMap::new(), None).unwrap();

    {
        let mut state = orch.state.write().await;
        state.running.insert(
            "1".into(),
            sympheo::orchestrator::state::RunningEntry {
                issue: make_issue("1", "TEST-1", "todo"),
                session: None,
                started_at: chrono::Utc::now(),
                retry_attempt: Some(2),
                turn_count: 0,
                cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                stagnation_counter: 0,
                last_state_change_at: chrono::Utc::now(),
            },
        );
        state.claimed.insert("1".into());
    }

    orch.handle_worker_exit("1", false, Some("failed".into())).await;

    let state = orch.state.read().await;
    assert!(!state.running.contains_key("1"));
    let retry = state.retry_attempts.get("1").unwrap();
    assert_eq!(retry.attempt, 3);
    assert_eq!(retry.error, Some("failed".into()));
}

#[tokio::test]
async fn test_orchestrator_process_retries_no_due() {
    let config = valid_config();
    let tracker = Arc::new(MockTracker {
        candidates: vec![],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, std::collections::HashMap::new(), None).unwrap();
    orch.process_retries().await;

    let state = orch.state.read().await;
    assert!(state.retry_attempts.is_empty());
}

#[tokio::test]
async fn test_orchestrator_process_retries_due_released() {
    let config = valid_config();
    let tracker = Arc::new(MockTracker {
        candidates: vec![],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, std::collections::HashMap::new(), None).unwrap();

    {
        let mut state = orch.state.write().await;
        state.claimed.insert("1".into());
        state.retry_attempts.insert(
            "1".into(),
            sympheo::tracker::model::RetryEntry {
                issue_id: "1".into(),
                identifier: "TEST-1".into(),
                attempt: 1,
                due_at: std::time::Instant::now(),
                error: Some("err".into()),
            },
        );
    }

    orch.process_retries().await;

    let state = orch.state.read().await;
    assert!(!state.claimed.contains("1"));
    assert!(!state.retry_attempts.contains_key("1"));
}

#[tokio::test]
async fn test_orchestrator_tick_dispatches_non_todo() {
    let config = valid_config();
    let issue = make_issue("1", "TEST-1", "in progress");
    let tracker = Arc::new(MockTracker {
        candidates: vec![issue.clone()],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, std::collections::HashMap::new(), None).unwrap();
    orch.tick().await;

    let state = orch.state.read().await;
    assert!(state.claimed.contains("1"));
    assert!(state.running.contains_key("1"));
}

#[tokio::test]
async fn test_orchestrator_tick_worker_completes() {
    let mut raw = valid_config().raw().clone();
    let mut codex = serde_yaml::Mapping::new();
    codex.insert(
        serde_yaml::Value::String("command".into()),
        serde_yaml::Value::String("false".into()),
    );
    raw.insert(
        serde_yaml::Value::String("codex".into()),
        serde_yaml::Value::Mapping(codex),
    );
    let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "prompt".into());
    let issue = make_issue("1", "TEST-1", "todo");
    let tracker = Arc::new(MockTracker {
        candidates: vec![issue.clone()],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, std::collections::HashMap::new(), None).unwrap();
    orch.tick().await;

    // Wait for worker to spawn and fail quickly
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let state = orch.state.read().await;
    assert!(!state.running.contains_key("1"));
    assert!(state.retry_attempts.contains_key("1"));
    let retry = state.retry_attempts.get("1").unwrap();
    assert!(retry.error.is_some());
}

#[tokio::test]
async fn test_orchestrator_tick_reconcile_stall_with_session() {
    let mut raw = valid_config().raw().clone();
    let mut codex = serde_yaml::Mapping::new();
    codex.insert(
        serde_yaml::Value::String("stall_timeout_ms".into()),
        serde_yaml::Value::Number(1.into()),
    );
    raw.insert(
        serde_yaml::Value::String("codex".into()),
        serde_yaml::Value::Mapping(codex),
    );
    let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "prompt".into());
    let tracker = Arc::new(MockTracker {
        candidates: vec![],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, std::collections::HashMap::new(), None).unwrap();

    {
        let mut state = orch.state.write().await;
        state.running.insert(
            "1".into(),
            sympheo::orchestrator::state::RunningEntry {
                issue: make_issue("1", "TEST-1", "in progress"),
                session: Some(sympheo::tracker::model::LiveSession {
                    session_id: "sess-1".into(),
                    thread_id: "sess-1".into(),
                    turn_id: "turn-1".into(),
                    agent_pid: None,
                    last_event: Some("turn_completed".into()),
                    last_timestamp: Some(chrono::Utc::now() - chrono::Duration::seconds(10)),
                    last_message: None,
                    input_tokens: 0,
                    output_tokens: 0,
                    total_tokens: 0,
                    last_reported_input_tokens: 0,
                    last_reported_output_tokens: 0,
                    last_reported_total_tokens: 0,
                    turn_count: 1,
                    pr_url: None,
                }),
                started_at: chrono::Utc::now(),
                retry_attempt: None,
                turn_count: 0,
                cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                stagnation_counter: 0,
                last_state_change_at: chrono::Utc::now(),
            },
        );
        state.claimed.insert("1".into());
    }

    orch.tick().await;

    let state = orch.state.read().await;
    assert!(!state.running.contains_key("1"));
}

#[tokio::test]
async fn test_orchestrator_tick_reconcile_terminal() {
    let config = valid_config();
    let issue = make_issue("1", "TEST-1", "closed");
    let tracker = Arc::new(MockTracker {
        candidates: vec![],
        by_states: vec![],
        by_ids: vec![issue.clone()],
    });
    let orch = Orchestrator::new(config, tracker, std::collections::HashMap::new(), None).unwrap();

    {
        let mut state = orch.state.write().await;
        state.running.insert(
            "1".into(),
            sympheo::orchestrator::state::RunningEntry {
                issue: make_issue("1", "TEST-1", "in progress"),
                session: None,
                started_at: chrono::Utc::now(),
                retry_attempt: None,
                turn_count: 0,
                cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                stagnation_counter: 0,
                last_state_change_at: chrono::Utc::now(),
            },
        );
        state.claimed.insert("1".into());
    }

    orch.tick().await;

    let state = orch.state.read().await;
    assert!(!state.running.contains_key("1"));
    assert!(!state.claimed.contains("1"));
}

#[tokio::test]
async fn test_orchestrator_tick_reconcile_active() {
    let config = valid_config();
    let issue = make_issue("1", "TEST-1", "in progress");
    let tracker = Arc::new(MockTracker {
        candidates: vec![],
        by_states: vec![],
        by_ids: vec![issue.clone()],
    });
    let orch = Orchestrator::new(config, tracker, std::collections::HashMap::new(), None).unwrap();

    {
        let mut state = orch.state.write().await;
        state.running.insert(
            "1".into(),
            sympheo::orchestrator::state::RunningEntry {
                issue: make_issue("1", "TEST-1", "todo"),
                session: None,
                started_at: chrono::Utc::now(),
                retry_attempt: None,
                turn_count: 0,
                cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                stagnation_counter: 0,
                last_state_change_at: chrono::Utc::now(),
            },
        );
        state.claimed.insert("1".into());
    }

    orch.tick().await;

    let state = orch.state.read().await;
    assert!(state.running.contains_key("1"));
    assert!(state.claimed.contains("1"));
}

#[tokio::test]
async fn test_orchestrator_tick_reconcile_unknown_state() {
    let config = valid_config();
    let issue = make_issue("1", "TEST-1", "unknown");
    let tracker = Arc::new(MockTracker {
        candidates: vec![],
        by_states: vec![],
        by_ids: vec![issue.clone()],
    });
    let orch = Orchestrator::new(config, tracker, std::collections::HashMap::new(), None).unwrap();

    {
        let mut state = orch.state.write().await;
        state.running.insert(
            "1".into(),
            sympheo::orchestrator::state::RunningEntry {
                issue: make_issue("1", "TEST-1", "todo"),
                session: None,
                started_at: chrono::Utc::now(),
                retry_attempt: None,
                turn_count: 0,
                cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                stagnation_counter: 0,
                last_state_change_at: chrono::Utc::now(),
            },
        );
        state.claimed.insert("1".into());
    }

    orch.tick().await;

    let state = orch.state.read().await;
    assert!(!state.running.contains_key("1"));
    assert!(!state.claimed.contains("1"));
}
