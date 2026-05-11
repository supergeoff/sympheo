use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use sympheo::config::typed::ServiceConfig;
use sympheo::error::SympheoError;
use sympheo::orchestrator::tick::Orchestrator;
use sympheo::tracker::IssueTracker;
use sympheo::tracker::model::Issue;

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

    async fn fetch_issues_by_states(&self, _states: &[String]) -> Result<Vec<Issue>, SympheoError> {
        Ok(self.by_states.clone())
    }

    async fn fetch_issue_states_by_ids(&self, _ids: &[String]) -> Result<Vec<Issue>, SympheoError> {
        Ok(self.by_ids.clone())
    }
}

fn valid_config() -> ServiceConfig {
    let mut raw = serde_json::Map::<String, serde_json::Value>::new();
    let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
    tracker.insert("kind".into(), serde_json::Value::String("github".into()));
    tracker.insert("api_key".into(), serde_json::Value::String("key".into()));
    tracker.insert(
        "project_slug".into(),
        serde_json::Value::String("owner/repo".into()),
    );
    tracker.insert("project_number".into(), serde_json::Value::Number(1.into()));
    raw.insert("tracker".into(), serde_json::Value::Object(tracker));
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
    let orch = Orchestrator::new(config, tracker, None).unwrap();
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
    let orch = Orchestrator::new(config, tracker, None).unwrap();
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
    let orch = Orchestrator::new(config, tracker, None).unwrap();
    orch.tick().await;

    let state = orch.state.read().await;
    assert!(!state.claimed.contains("1"));
    assert!(!state.running.contains_key("1"));
}

#[tokio::test]
async fn test_orchestrator_tick_dispatches_todo_with_terminal_blockers() {
    // SPEC §17.5: "An issue in the first active state with terminal blockers
    // is eligible". Companion to `test_orchestrator_tick_skips_blocked_todo`:
    // same issue shape, but every blocker has reached a terminal state, so
    // dispatch must claim and run it.
    let config = valid_config();
    let issue = Issue {
        blocked_by: vec![
            sympheo::tracker::model::BlockerRef {
                id: Some("2".into()),
                identifier: Some("TEST-2".into()),
                state: Some("closed".into()),
            },
            sympheo::tracker::model::BlockerRef {
                id: Some("3".into()),
                identifier: Some("TEST-3".into()),
                state: Some("done".into()),
            },
        ],
        ..make_issue("1", "TEST-1", "todo")
    };
    let tracker = Arc::new(MockTracker {
        candidates: vec![issue],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, None).unwrap();
    orch.tick().await;

    let state = orch.state.read().await;
    assert!(
        state.claimed.contains("1"),
        "todo with all-terminal blockers must be claimed"
    );
    assert!(
        state.running.contains_key("1"),
        "todo with all-terminal blockers must be running"
    );
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
    let orch = Orchestrator::new(config, tracker, None).unwrap();
    orch.tick().await;

    let state = orch.state.read().await;
    assert!(!state.claimed.contains("1"));
}

#[tokio::test]
async fn test_orchestrator_tick_respects_concurrency_limit() {
    let mut config = valid_config();
    config = ServiceConfig::new(config.raw().clone(), PathBuf::from("/tmp"), "prompt".into());
    let mut raw = config.raw().clone();
    let mut agent = serde_json::Map::<String, serde_json::Value>::new();
    agent.insert(
        "max_concurrent_agents".into(),
        serde_json::Value::Number(1.into()),
    );
    raw.insert("agent".into(), serde_json::Value::Object(agent));
    let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "prompt".into());

    let tracker = Arc::new(MockTracker {
        candidates: vec![
            make_issue("1", "TEST-1", "todo"),
            make_issue("2", "TEST-2", "todo"),
        ],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, None).unwrap();
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
    let orch = Orchestrator::new(config, tracker, None).unwrap();

    let mut new_raw = serde_json::Map::<String, serde_json::Value>::new();
    let mut polling = serde_json::Map::<String, serde_json::Value>::new();
    polling.insert(
        "interval_ms".into(),
        serde_json::Value::Number(10000.into()),
    );
    new_raw.insert("polling".into(), serde_json::Value::Object(polling));
    let mut agent = serde_json::Map::<String, serde_json::Value>::new();
    agent.insert(
        "max_concurrent_agents".into(),
        serde_json::Value::Number(5.into()),
    );
    new_raw.insert("agent".into(), serde_json::Value::Object(agent));
    let new_config = ServiceConfig::new(new_raw, PathBuf::from("/tmp"), "prompt".into());

    orch.reload_config(new_config).await;

    let state = orch.state.read().await;
    assert_eq!(state.poll_interval_ms, 10000);
    assert_eq!(state.max_concurrent_agents, 5);
}

#[tokio::test]
async fn test_orchestrator_invalid_reload_keeps_last_known_good() {
    // SPEC §17.1: "Invalid workflow reload keeps last known good effective
    // configuration and emits an operator-visible error".
    //
    // The watcher in src/main.rs guards reload by short-circuiting on a
    // loader error. This test exercises the same contract end-to-end:
    //   1. Successfully load a good workflow → reload_config applies.
    //   2. A subsequent invalid workflow file MUST surface a typed error
    //      from `WorkflowLoader::load()` and the in-memory state MUST stay
    //      pinned at the last good values (no partial / silent overwrite).
    use std::path::PathBuf;
    use sympheo::workflow::loader::WorkflowLoader;

    let tracker = Arc::new(MockTracker {
        candidates: vec![],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(valid_config(), tracker, None).unwrap();

    // 1) Land a known-good config.
    let good_path = std::env::temp_dir().join(format!(
        "sympheo_reload_good_{}_{}.md",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(
        &good_path,
        r#"---
polling:
  interval_ms: 11111
agent:
  max_concurrent_agents: 7
---
ok
"#,
    )
    .expect("write good workflow");
    let good_loaded = WorkflowLoader::new(Some(good_path.clone()))
        .load()
        .expect("good workflow loads");
    let good_cfg = ServiceConfig::new(
        good_loaded.config,
        PathBuf::from("/tmp"),
        good_loaded.prompt_template,
    );
    orch.reload_config(good_cfg).await;
    {
        let st = orch.state.read().await;
        assert_eq!(st.poll_interval_ms, 11111);
        assert_eq!(st.max_concurrent_agents, 7);
    }

    // 2) Now write an INVALID workflow (unclosed front matter) at a fresh
    //    path and try to load it. The loader must return a typed error and
    //    the orchestrator state must stay at the last-known-good values.
    let bad_path = std::env::temp_dir().join(format!(
        "sympheo_reload_bad_{}_{}.md",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(&bad_path, "---\ntracker: kind\nDo work").expect("write bad workflow");
    let bad_result = WorkflowLoader::new(Some(bad_path.clone())).load();
    assert!(
        matches!(bad_result, Err(SympheoError::WorkflowParseError(_))),
        "invalid workflow must surface typed parse error, got {:?}",
        bad_result
    );

    // The watcher path skips reload on Err, so we mirror that here: do NOT
    // call reload_config. Assert the orchestrator still holds the last good
    // effective configuration.
    {
        let st = orch.state.read().await;
        assert_eq!(
            st.poll_interval_ms, 11111,
            "invalid reload must not change poll_interval_ms"
        );
        assert_eq!(
            st.max_concurrent_agents, 7,
            "invalid reload must not change max_concurrent_agents"
        );
    }

    let _ = std::fs::remove_file(&good_path);
    let _ = std::fs::remove_file(&bad_path);
}

#[tokio::test]
async fn test_orchestrator_handle_worker_exit_normal() {
    let config = valid_config();
    let tracker = Arc::new(MockTracker {
        candidates: vec![],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, None).unwrap();

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
    let orch = Orchestrator::new(config, tracker, None).unwrap();

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

    orch.handle_worker_exit("1", false, Some("failed".into()))
        .await;

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
    let orch = Orchestrator::new(config, tracker, None).unwrap();
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
    let orch = Orchestrator::new(config, tracker, None).unwrap();

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
    let orch = Orchestrator::new(config, tracker, None).unwrap();
    orch.tick().await;

    let state = orch.state.read().await;
    assert!(state.claimed.contains("1"));
    assert!(state.running.contains_key("1"));
}

#[tokio::test]
async fn test_orchestrator_tick_worker_completes() {
    let mut raw = valid_config().raw().clone();
    let mut cli = serde_json::Map::<String, serde_json::Value>::new();
    // Use the spec-recognized "opencode" leading binary so validate_for_dispatch
    // (§6.3 + §10.1) accepts the command. opencode is unlikely to be on PATH in CI;
    // if absent → bash exits 127; if present → the bogus flag causes a non-zero exit.
    // Either path triggers the subprocess-failure → retry-queue path under test.
    cli.insert(
        "command".into(),
        serde_json::Value::String("opencode --sympheo-test-fail-Q9zXp".into()),
    );
    raw.insert("cli".into(), serde_json::Value::Object(cli));
    let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "prompt".into());
    let issue = make_issue("1", "TEST-1", "todo");
    let tracker = Arc::new(MockTracker {
        candidates: vec![issue.clone()],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, None).unwrap();
    orch.tick().await;

    // Wait for worker to spawn and fail (subprocess returns non-zero or cannot be found)
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        let st = orch.state.read().await;
        if !st.running.contains_key("1") && st.retry_attempts.contains_key("1") {
            assert!(st.retry_attempts.get("1").unwrap().error.is_some());
            return;
        }
        if std::time::Instant::now() > deadline {
            panic!(
                "worker did not transition to retry within 5s; running={:?} retry_keys={:?}",
                st.running.keys().collect::<Vec<_>>(),
                st.retry_attempts.keys().collect::<Vec<_>>()
            );
        }
    }
}

#[tokio::test]
async fn test_orchestrator_tick_reconcile_stall_with_session() {
    let mut raw = valid_config().raw().clone();
    let mut cli = serde_json::Map::<String, serde_json::Value>::new();
    cli.insert(
        "stall_timeout_ms".into(),
        serde_json::Value::Number(1.into()),
    );
    raw.insert("cli".into(), serde_json::Value::Object(cli));
    let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "prompt".into());
    let tracker = Arc::new(MockTracker {
        candidates: vec![],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, None).unwrap();

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
    let orch = Orchestrator::new(config, tracker, None).unwrap();

    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
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
                cancelled: cancelled.clone(),
                stagnation_counter: 0,
                last_state_change_at: chrono::Utc::now(),
            },
        );
        state.claimed.insert("1".into());
    }

    orch.tick().await;

    // Reconcile-terminal now only signals the worker via `cancelled` — the
    // spawn-task wrapper around `run_worker` is the single owner of removing
    // the entry from `running`/`claimed` and tearing the workspace down. The
    // entry stays in place until the wrapper fires; in this test there is no
    // real worker, so the entry remains and only `cancelled` is asserted.
    assert!(cancelled.load(std::sync::atomic::Ordering::Relaxed));
    let state = orch.state.read().await;
    assert!(state.running.contains_key("1"));
    assert!(state.claimed.contains("1"));
}

#[tokio::test]
async fn test_orchestrator_process_retries_due_re_dispatches() {
    let config = valid_config();
    let issue = make_issue("1", "TEST-1", "todo");
    let tracker = Arc::new(MockTracker {
        candidates: vec![issue.clone()],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, None).unwrap();

    {
        let mut state = orch.state.write().await;
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
        state.claimed.insert("1".into());
    }

    orch.process_retries().await;

    let state = orch.state.read().await;
    assert!(state.running.contains_key("1"));
    assert!(state.claimed.contains("1"));
    assert!(!state.retry_attempts.contains_key("1"));
}

// Custom tracker that returns error on fetch_candidate_issues
struct FailingTracker;

#[async_trait]
impl IssueTracker for FailingTracker {
    async fn fetch_candidate_issues(
        &self,
    ) -> Result<Vec<sympheo::tracker::model::Issue>, sympheo::error::SympheoError> {
        Err(sympheo::error::SympheoError::TrackerApiRequest(
            "boom".into(),
        ))
    }
    async fn fetch_issues_by_states(
        &self,
        _states: &[String],
    ) -> Result<Vec<sympheo::tracker::model::Issue>, sympheo::error::SympheoError> {
        Ok(vec![])
    }
    async fn fetch_issue_states_by_ids(
        &self,
        _ids: &[String],
    ) -> Result<Vec<sympheo::tracker::model::Issue>, sympheo::error::SympheoError> {
        Ok(vec![])
    }
}

#[tokio::test]
async fn test_orchestrator_process_retries_fetch_fails() {
    let config = valid_config();
    let tracker: Arc<dyn IssueTracker> = Arc::new(FailingTracker);
    let orch = Orchestrator::new(config, tracker, None).unwrap();

    {
        let mut state = orch.state.write().await;
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
        state.claimed.insert("1".into());
    }

    orch.process_retries().await;

    let state = orch.state.read().await;
    assert!(state.retry_attempts.contains_key("1"));
    assert!(state.claimed.contains("1"));
}

#[tokio::test]
async fn test_orchestrator_process_retries_fetch_fails_max_attempts() {
    let config = valid_config();
    let tracker: Arc<dyn IssueTracker> = Arc::new(FailingTracker);
    let orch = Orchestrator::new(config, tracker, None).unwrap();

    {
        let mut state = orch.state.write().await;
        state.retry_attempts.insert(
            "1".into(),
            sympheo::tracker::model::RetryEntry {
                issue_id: "1".into(),
                identifier: "TEST-1".into(),
                attempt: 5,
                due_at: std::time::Instant::now(),
                error: Some("err".into()),
            },
        );
        state.claimed.insert("1".into());
    }

    orch.process_retries().await;

    let state = orch.state.read().await;
    assert!(!state.retry_attempts.contains_key("1"));
    assert!(!state.claimed.contains("1"));
}

#[tokio::test]
async fn test_orchestrator_process_retries_no_slots() {
    let mut raw = valid_config().raw().clone();
    let mut agent = serde_json::Map::<String, serde_json::Value>::new();
    agent.insert(
        "max_concurrent_agents".into(),
        serde_json::Value::Number(1.into()),
    );
    raw.insert("agent".into(), serde_json::Value::Object(agent));
    let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "prompt".into());
    let issue = make_issue("1", "TEST-1", "todo");
    let tracker: Arc<dyn IssueTracker> = Arc::new(MockTracker {
        candidates: vec![issue.clone()],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, None).unwrap();

    {
        let mut state = orch.state.write().await;
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
        state.claimed.insert("1".into());
        // Fill the only slot so no slots are available
        state.running.insert(
            "2".into(),
            sympheo::orchestrator::state::RunningEntry {
                issue: make_issue("2", "TEST-2", "todo"),
                session: None,
                started_at: chrono::Utc::now(),
                retry_attempt: None,
                turn_count: 0,
                cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                stagnation_counter: 0,
                last_state_change_at: chrono::Utc::now(),
            },
        );
    }

    orch.process_retries().await;

    let state = orch.state.read().await;
    assert!(state.retry_attempts.contains_key("1"));
    assert!(state.claimed.contains("1"));
}

#[tokio::test]
async fn test_orchestrator_process_retries_terminal_state() {
    let config = valid_config();
    let issue = make_issue("1", "TEST-1", "closed");
    let tracker: Arc<dyn IssueTracker> = Arc::new(MockTracker {
        candidates: vec![issue.clone()],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, None).unwrap();

    {
        let mut state = orch.state.write().await;
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
        state.claimed.insert("1".into());
    }

    orch.process_retries().await;

    let state = orch.state.read().await;
    assert!(!state.retry_attempts.contains_key("1"));
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
    let orch = Orchestrator::new(config, tracker, None).unwrap();

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
    let orch = Orchestrator::new(config, tracker, None).unwrap();

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

#[tokio::test]
async fn test_orchestrator_process_retries_max_attempts_reached() {
    let mut raw = valid_config().raw().clone();
    let mut cli = serde_json::Map::<String, serde_json::Value>::new();
    // SPEC §10.1: cli.command must select a known adapter. We use the bundled
    // `mock-cli` adapter pointed at a missing script so the worker fails fast
    // and deterministically (MockBackend reads the script at run_turn and
    // surfaces an AgentRunnerError on ENOENT). This isolates the test from
    // whatever real agent CLIs (`opencode`, `pi`, ...) happen to be installed
    // on the host — the previous `opencode run` made this race-prone.
    cli.insert(
        "command".into(),
        serde_json::Value::String("mock-cli".into()),
    );
    let mut opts = serde_json::Map::<String, serde_json::Value>::new();
    opts.insert(
        "script".into(),
        serde_json::Value::String("/tmp/sympheo-test-nonexistent-script.yaml".into()),
    );
    cli.insert("options".into(), serde_json::Value::Object(opts));
    raw.insert("cli".into(), serde_json::Value::Object(cli));
    let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "prompt".into());
    let issue = make_issue("1", "TEST-1", "todo");
    let tracker: Arc<dyn IssueTracker> = Arc::new(MockTracker {
        candidates: vec![issue.clone()],
        by_states: vec![],
        by_ids: vec![],
    });
    let orch = Orchestrator::new(config, tracker, None).unwrap();

    {
        let mut state = orch.state.write().await;
        state.retry_attempts.insert(
            "1".into(),
            sympheo::tracker::model::RetryEntry {
                issue_id: "1".into(),
                identifier: "TEST-1".into(),
                attempt: 5,
                due_at: std::time::Instant::now(),
                error: Some("err".into()),
            },
        );
        state.claimed.insert("1".into());
    }

    orch.process_retries().await;
    // Poll until the spawned worker has been cleaned up, with a generous
    // overall budget. `mock-cli` + missing-script path fails in milliseconds,
    // but a fixed sleep is fragile under load.
    let deadline = std::time::Instant::now() + tokio::time::Duration::from_secs(5);
    loop {
        let state = orch.state.read().await;
        if !state.running.contains_key("1") {
            break;
        }
        drop(state);
        if std::time::Instant::now() >= deadline {
            panic!("worker did not exit within 5s");
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    let state = orch.state.read().await;
    assert!(!state.running.contains_key("1"));
    assert!(!state.claimed.contains("1"));
    assert!(!state.retry_attempts.contains_key("1"));
}
