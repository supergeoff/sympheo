use std::path::PathBuf;
use sympheo::config::typed::ServiceConfig;
use sympheo::tracker::model::Issue;
use sympheo::workspace::manager::WorkspaceManager;

#[test]
fn test_workflow_loader_with_front_matter() {
    let content = r#"---
tracker:
  kind: github
  project_slug: test/repo
---
Do the work
"#;
    let wf = sympheo::workflow::parser::parse(content).unwrap();
    assert!(!wf.config.is_empty());
    assert_eq!(wf.prompt_template, "Do the work");
}

#[test]
fn test_service_config_defaults() {
    let raw = serde_json::Map::<String, serde_json::Value>::new();
    let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "prompt".into());
    assert_eq!(config.tracker_kind(), None);
    assert_eq!(config.poll_interval_ms(), 30000);
    assert_eq!(config.max_concurrent_agents(), 10);
    assert_eq!(config.max_turns(), 20);
    assert_eq!(config.cli_command(), "opencode run");
    // SPEC §5.3.6 default: 300000 (5 min)
    assert_eq!(config.cli_stall_timeout_ms(), 300000);
}

#[test]
fn test_workspace_sanitization() {
    // SPEC §4.2 + §9.2: chars outside [A-Za-z0-9._-] become '-'
    assert_eq!(WorkspaceManager::sanitize_identifier("ABC-123"), "ABC-123");
    assert_eq!(
        WorkspaceManager::sanitize_identifier("sympheo#42"),
        "sympheo-42"
    );
    assert_eq!(
        WorkspaceManager::sanitize_identifier("feat/new_thing"),
        "feat-new_thing"
    );
    assert_eq!(
        WorkspaceManager::sanitize_identifier("bug: crash!"),
        "bug--crash-"
    );
}

#[test]
fn test_issue_is_blocked() {
    let issue = Issue {
        id: "1".into(),
        identifier: "TEST-1".into(),
        title: "test".into(),
        description: None,
        priority: None,
        state: "todo".into(),
        branch_name: None,
        url: None,
        labels: vec![],
        blocked_by: vec![sympheo::tracker::model::BlockerRef {
            id: Some("2".into()),
            identifier: Some("TEST-2".into()),
            state: Some("in progress".into()),
        }],
        ..Default::default()
    };
    let terminal = vec!["closed".into(), "done".into()];
    assert!(issue.is_blocked(&terminal));

    let unblocked = Issue {
        blocked_by: vec![sympheo::tracker::model::BlockerRef {
            id: Some("2".into()),
            identifier: Some("TEST-2".into()),
            state: Some("closed".into()),
        }],
        ..issue
    };
    assert!(!unblocked.is_blocked(&terminal));
}

#[test]
fn test_config_var_resolution() {
    unsafe { std::env::set_var("TEST_SYM_KEY", "secret123") };
    assert_eq!(
        sympheo::config::resolver::resolve_value("$TEST_SYM_KEY"),
        "secret123"
    );
    assert_eq!(sympheo::config::resolver::resolve_value("plain"), "plain");
}

#[test]
fn test_daytona_config_parsing() {
    use std::path::PathBuf;
    let mut raw = serde_json::Map::<String, serde_json::Value>::new();
    let mut daytona = serde_json::Map::<String, serde_json::Value>::new();
    daytona.insert("enabled".into(), serde_json::Value::Bool(true));
    daytona.insert(
        "api_key".into(),
        serde_json::Value::String("$DAYTONA_KEY".into()),
    );
    daytona.insert(
        "endpoint".into(),
        serde_json::Value::String("https://api.daytona.io".into()),
    );
    raw.insert("daytona".into(), serde_json::Value::Object(daytona));
    unsafe { std::env::set_var("DAYTONA_KEY", "secret") };
    let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "prompt".into());
    assert!(config.daytona_enabled());
    assert_eq!(config.daytona_api_key(), Some("secret".into()));
    assert_eq!(config.daytona_api_url(), "https://api.daytona.io");
}

#[cfg(test)]
mod workstream0_tests {
    use chrono::Utc;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use sympheo::orchestrator::state::{OrchestratorState, RunningEntry};
    use sympheo::tracker::model::{Issue, LiveSession};

    fn dummy_issue(
        id: &str,
        priority: Option<i32>,
        created_at: Option<chrono::DateTime<Utc>>,
    ) -> Issue {
        Issue {
            id: id.into(),
            identifier: format!("TEST-{id}"),
            title: "test".into(),
            description: None,
            priority,
            state: "todo".into(),
            branch_name: None,
            url: None,
            labels: vec![],
            blocked_by: vec![],
            created_at,
            ..Default::default()
        }
    }

    #[test]
    fn test_token_delta_no_double_count() {
        let mut state = OrchestratorState::new(30000, 10);
        let issue = dummy_issue("1", None, None);
        state.running.insert(
            "1".into(),
            RunningEntry {
                issue: issue.clone(),
                session: Some(LiveSession {
                    session_id: "s1".into(),
                    thread_id: "t1".into(),
                    turn_id: "turn-1".into(),
                    agent_pid: None,
                    last_event: None,
                    last_timestamp: None,
                    last_message: None,
                    input_tokens: 100,
                    output_tokens: 50,
                    total_tokens: 150,
                    last_reported_input_tokens: 100,
                    last_reported_output_tokens: 50,
                    last_reported_total_tokens: 150,
                    turn_count: 1,
                    pr_url: None,
                }),
                started_at: Utc::now(),
                retry_attempt: None,
                turn_count: 1,
                cancelled: Arc::new(AtomicBool::new(false)),
                stagnation_counter: 0,
                last_state_change_at: Utc::now(),
            },
        );

        // Simulate second turn with same totals
        if let Some(entry) = state.running.get_mut("1")
            && let Some(ref mut sess) = entry.session
        {
            let new_input: u64 = 100;
            let new_output: u64 = 50;
            let new_total: u64 = 150;
            let delta_input = new_input.saturating_sub(sess.last_reported_input_tokens);
            let delta_output = new_output.saturating_sub(sess.last_reported_output_tokens);
            let delta_total = new_total.saturating_sub(sess.last_reported_total_tokens);
            state.cli_totals.input_tokens += delta_input;
            state.cli_totals.output_tokens += delta_output;
            state.cli_totals.total_tokens += delta_total;
            sess.last_reported_input_tokens = new_input;
            sess.last_reported_output_tokens = new_output;
            sess.last_reported_total_tokens = new_total;
        }

        assert_eq!(state.cli_totals.input_tokens, 0);
        assert_eq!(state.cli_totals.output_tokens, 0);
        assert_eq!(state.cli_totals.total_tokens, 0);
    }

    #[test]
    fn test_token_delta_accumulation() {
        let mut state = OrchestratorState::new(30000, 10);
        let issue = dummy_issue("1", None, None);
        state.running.insert(
            "1".into(),
            RunningEntry {
                issue: issue.clone(),
                session: Some(LiveSession {
                    session_id: "s1".into(),
                    thread_id: "t1".into(),
                    turn_id: "turn-1".into(),
                    agent_pid: None,
                    last_event: None,
                    last_timestamp: None,
                    last_message: None,
                    input_tokens: 100,
                    output_tokens: 50,
                    total_tokens: 150,
                    last_reported_input_tokens: 100,
                    last_reported_output_tokens: 50,
                    last_reported_total_tokens: 150,
                    turn_count: 1,
                    pr_url: None,
                }),
                started_at: Utc::now(),
                retry_attempt: None,
                turn_count: 1,
                cancelled: Arc::new(AtomicBool::new(false)),
                stagnation_counter: 0,
                last_state_change_at: Utc::now(),
            },
        );

        // Simulate second turn with higher totals
        if let Some(entry) = state.running.get_mut("1")
            && let Some(ref mut sess) = entry.session
        {
            let new_input: u64 = 200;
            let new_output: u64 = 100;
            let new_total: u64 = 300;
            let delta_input = new_input.saturating_sub(sess.last_reported_input_tokens);
            let delta_output = new_output.saturating_sub(sess.last_reported_output_tokens);
            let delta_total = new_total.saturating_sub(sess.last_reported_total_tokens);
            state.cli_totals.input_tokens += delta_input;
            state.cli_totals.output_tokens += delta_output;
            state.cli_totals.total_tokens += delta_total;
            sess.last_reported_input_tokens = new_input;
            sess.last_reported_output_tokens = new_output;
            sess.last_reported_total_tokens = new_total;
        }

        assert_eq!(state.cli_totals.input_tokens, 100);
        assert_eq!(state.cli_totals.output_tokens, 50);
        assert_eq!(state.cli_totals.total_tokens, 150);
    }

    #[test]
    fn test_dispatch_sort_order() {
        let now = Utc::now();
        let issues = vec![
            dummy_issue("low", Some(3), Some(now)),
            dummy_issue("high", Some(1), Some(now)),
            dummy_issue("mid", Some(2), Some(now)),
        ];

        let mut sorted = issues;
        sorted.sort_by(|a, b| {
            a.priority
                .unwrap_or(i32::MAX)
                .cmp(&b.priority.unwrap_or(i32::MAX))
                .then_with(|| a.created_at.cmp(&b.created_at))
                .then_with(|| a.identifier.cmp(&b.identifier))
        });

        assert_eq!(sorted[0].id, "high");
        assert_eq!(sorted[1].id, "mid");
        assert_eq!(sorted[2].id, "low");
    }
}
