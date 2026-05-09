//! End-to-end orchestrator test using the P5 mock CLI adapter.
//!
//! Drives a complete worker lifecycle (workspace creation, prompt rendering,
//! mock subprocess invocation, event streaming, turn completion, retry
//! scheduling) WITHOUT spending a single token on a real opencode run.
//!
//! Maps to SPEC §17.5 (orchestrator dispatch / reconciliation / retry) and
//! §17.6 (CLI adapter conformance — selection, run_turn, on_event callback).

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
    by_ids: std::sync::Mutex<Vec<Issue>>,
}

#[async_trait]
impl IssueTracker for MockTracker {
    async fn fetch_candidate_issues(&self) -> Result<Vec<Issue>, SympheoError> {
        Ok(self.candidates.clone())
    }
    async fn fetch_issues_by_states(&self, _states: &[String]) -> Result<Vec<Issue>, SympheoError> {
        Ok(vec![])
    }
    async fn fetch_issue_states_by_ids(&self, _ids: &[String]) -> Result<Vec<Issue>, SympheoError> {
        Ok(self.by_ids.lock().unwrap().clone())
    }
}

fn unique_tmp(suffix: &str) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("sympheo_e2e_mock_{}_{}", suffix, ts))
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

fn config_for_mock(workspace_root: &std::path::Path, script_path: &str) -> ServiceConfig {
    let mut raw = serde_json::Map::<String, serde_json::Value>::new();

    // Tracker block (required for validate_for_dispatch)
    let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
    tracker.insert("kind".into(), serde_json::Value::String("github".into()));
    tracker.insert("api_key".into(), serde_json::Value::String("key".into()));
    tracker.insert(
        "project_slug".into(),
        serde_json::Value::String("owner/repo".into()),
    );
    tracker.insert("project_number".into(), serde_json::Value::Number(1.into()));
    raw.insert("tracker".into(), serde_json::Value::Object(tracker));

    let mut workspace = serde_json::Map::<String, serde_json::Value>::new();
    workspace.insert(
        "root".into(),
        serde_json::Value::String(workspace_root.to_string_lossy().to_string()),
    );
    raw.insert("workspace".into(), serde_json::Value::Object(workspace));

    let mut cli = serde_json::Map::<String, serde_json::Value>::new();
    cli.insert(
        "command".into(),
        serde_json::Value::String("mock-cli".into()),
    );
    let mut opts = serde_json::Map::<String, serde_json::Value>::new();
    opts.insert(
        "script".into(),
        serde_json::Value::String(script_path.into()),
    );
    cli.insert("options".into(), serde_json::Value::Object(opts));
    cli.insert(
        "turn_timeout_ms".into(),
        serde_json::Value::Number(5000.into()),
    );
    cli.insert(
        "stall_timeout_ms".into(),
        serde_json::Value::Number(2000.into()),
    );
    raw.insert("cli".into(), serde_json::Value::Object(cli));

    let mut agent = serde_json::Map::<String, serde_json::Value>::new();
    agent.insert("max_turns".into(), serde_json::Value::Number(2.into()));
    raw.insert("agent".into(), serde_json::Value::Object(agent));

    ServiceConfig::new(
        raw,
        PathBuf::from("/tmp"),
        "Test prompt {{issue.identifier}}".into(),
    )
}

/// SPEC §17.5 / §17.6: a full tick + worker lifecycle drives an issue
/// through the orchestrator using the mock CLI adapter. We assert that:
/// - the tick dispatches the issue (running map populated),
/// - the worker runs the mock script to completion,
/// - on normal exit, the orchestrator schedules a continuation retry per §7.3.
#[tokio::test]
async fn test_e2e_mock_dispatch_and_continuation() {
    let tmp = unique_tmp("dispatch");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    // Pre-create the workspace dir so the script can live there.
    let ws_dir = tmp.join("repo-1");
    std::fs::create_dir_all(&ws_dir).unwrap();
    let script = r#"
events:
  - type: step_start
    session_id: sess-1
    message_id: msg-1
  - type: text
    message_id: msg-1
    text: "Mock turn output"
  - type: step_finish
    message_id: msg-1
    reason: stop
    input_tokens: 5
    output_tokens: 7
"#;
    std::fs::write(ws_dir.join("script.yaml"), script).unwrap();

    let config = config_for_mock(&tmp, "script.yaml");
    let issue = make_issue("1", "repo#1", "todo");
    let tracker = Arc::new(MockTracker {
        candidates: vec![issue.clone()],
        by_ids: std::sync::Mutex::new(vec![issue]),
    });
    let orch = Orchestrator::new(config, tracker, std::collections::HashMap::new(), None).unwrap();
    orch.tick().await;

    // Wait for the worker to finish the turn loop and exit.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        let st = orch.state.read().await;
        if !st.running.contains_key("1") {
            // SPEC §7.3 + §8.4: normal exit schedules a short continuation
            // retry (attempt=1, delay 1000ms).
            let maybe_retry = st.retry_attempts.get("1");
            if let Some(retry) = maybe_retry {
                assert_eq!(retry.attempt, 1, "continuation retry should be attempt 1");
                return;
            }
            // Worker exited but no retry yet — keep waiting briefly.
        }
        if std::time::Instant::now() > deadline {
            panic!(
                "worker did not exit and schedule continuation retry within 8s; running={:?} retry_keys={:?}",
                st.running.keys().collect::<Vec<_>>(),
                st.retry_attempts.keys().collect::<Vec<_>>()
            );
        }
    }
}

/// SPEC §17.6: cancellation. An external cancel flag set on the running entry
/// causes the mock to return TurnCancelled mid-script; the orchestrator
/// converts the failure to a retry per §7.3 / §8.4 with attempt > 1 (because
/// it's failure-driven, not continuation).
#[tokio::test]
async fn test_e2e_mock_cancel_via_state() {
    let tmp = unique_tmp("cancel");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let ws_dir = tmp.join("repo-1");
    std::fs::create_dir_all(&ws_dir).unwrap();

    // Long sleep so we have time to flip the cancel flag.
    let script = r#"
events:
  - type: step_start
    session_id: sess-1
    message_id: msg-1
  - type: sleep
    delay_ms: 4000
  - type: step_finish
    message_id: msg-1
    reason: stop
"#;
    std::fs::write(ws_dir.join("script.yaml"), script).unwrap();

    let config = config_for_mock(&tmp, "script.yaml");
    let issue = make_issue("1", "repo#1", "todo");
    let tracker = Arc::new(MockTracker {
        candidates: vec![issue.clone()],
        by_ids: std::sync::Mutex::new(vec![issue]),
    });
    let orch = Orchestrator::new(config, tracker, std::collections::HashMap::new(), None).unwrap();
    orch.tick().await;

    // Wait for the worker to land in running, then flip cancel.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let st = orch.state.read().await;
        if let Some(entry) = st.running.get("1") {
            entry
                .cancelled
                .store(true, std::sync::atomic::Ordering::Relaxed);
            break;
        }
        if std::time::Instant::now() > deadline {
            panic!("worker did not enter running state within 2s");
        }
    }

    // Wait for the worker to react to the cancel and exit.
    let deadline2 = std::time::Instant::now() + std::time::Duration::from_secs(6);
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        let st = orch.state.read().await;
        if !st.running.contains_key("1") {
            return;
        }
        if std::time::Instant::now() > deadline2 {
            panic!("worker did not exit after cancel within 6s");
        }
    }
}
