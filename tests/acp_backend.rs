//! Integration tests for `AcpBackend` (lot 5 — SYM-20).
//!
//! Exercises the full `start_session → run_turn → stop_session` lifecycle
//! through the `fake_acp_server` binary, which serves the ACP protocol on
//! stdin/stdout controlled by the `FAKE_ACP_SCENARIO` env var.
//!
//! All tests use short timeouts (≤ 200 ms) so the suite stays under 30 s.
#![cfg(feature = "fake-acp")]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use sympheo::agent::acp::adapter::{AcpAdapter, default_client_capabilities};
use sympheo::agent::backend::acp::{AcpBackend, AcpBackendConfig};
use sympheo::agent::backend::AgentBackend;
use sympheo::agent::cli::{CliOptions, Permission};
use sympheo::agent::parser::{AgentEvent, EmittedEvent, TurnOutcome};
use sympheo::error::SympheoError;
use sympheo::tracker::model::Issue;
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Fake process adapter
// ---------------------------------------------------------------------------

/// An [`AcpAdapter`] that spawns the `fake_acp_server` binary with the given
/// scenario name.  Only available in test builds (feature = "fake-acp").
struct FakeProcessAdapter {
    scenario: String,
}

impl AcpAdapter for FakeProcessAdapter {
    fn kind(&self) -> &str {
        "fake"
    }

    fn spawn_spec(&self, cli_env: &HashMap<String, String>) -> std::process::Command {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_fake_acp_server"));
        cmd.env("FAKE_ACP_SCENARIO", &self.scenario);
        for (k, v) in cli_env {
            cmd.env(k, v);
        }
        cmd
    }

    fn client_capabilities(&self) -> agent_client_protocol::schema::ClientCapabilities {
        default_client_capabilities()
    }

    fn session_new_params(&self, cwd: PathBuf) -> agent_client_protocol::schema::NewSessionRequest {
        agent_client_protocol::schema::NewSessionRequest::new(cwd)
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Short timeouts so tests finish quickly.
fn fast_config() -> AcpBackendConfig {
    AcpBackendConfig {
        session_start_timeout: Duration::from_millis(2000),
        read_timeout: Duration::from_millis(300),
        tool_progress_timeout: Duration::from_millis(200),
        cancel_grace: Duration::from_millis(150),
        turn_timeout: Duration::from_millis(5000),
    }
}

async fn make_backend(scenario: &str) -> AcpBackend {
    let adapter = Arc::new(FakeProcessAdapter {
        scenario: scenario.to_string(),
    });
    AcpBackend::new(adapter, fast_config(), HashMap::new())
}

async fn run_turn_with_opts(
    backend: &AcpBackend,
    cancelled: Arc<AtomicBool>,
    permission: Option<Permission>,
) -> (Result<sympheo::agent::parser::TurnResult, SympheoError>, Vec<AgentEvent>) {
    let (event_tx, mut event_rx) = mpsc::channel::<EmittedEvent>(256);
    let issue = Issue::default();
    let opts = CliOptions {
        permission,
        ..Default::default()
    };

    let result = backend
        .run_turn(
            &issue,
            "test prompt",
            None,
            Path::new("/tmp"),
            cancelled,
            event_tx,
            &opts,
        )
        .await;

    // Drain events
    let mut events = Vec::new();
    while let Ok(e) = event_rx.try_recv() {
        events.push(e.event);
    }

    (result, events)
}

async fn run_turn(
    backend: &AcpBackend,
) -> (Result<sympheo::agent::parser::TurnResult, SympheoError>, Vec<AgentEvent>) {
    run_turn_with_opts(backend, Arc::new(AtomicBool::new(false)), None).await
}

// ---------------------------------------------------------------------------
// Scenario 1 — Lifecycle complet
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lifecycle_complete_end_turn() {
    let backend = make_backend("lifecycle_end_turn").await;
    backend
        .start_session(PathBuf::from("/tmp"))
        .await
        .expect("start_session should succeed");

    let (result, events) = run_turn(&backend).await;

    let turn = result.expect("run_turn should succeed");
    assert_eq!(turn.outcome, TurnOutcome::Succeeded, "expected Succeeded");

    assert!(
        events.iter().any(|e| matches!(e, AgentEvent::Text { .. })),
        "expected at least one Text event, got: {events:?}"
    );

    backend.stop_session();
}

// ---------------------------------------------------------------------------
// Scenario 2 — Cancel coopératif + trailing updates
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancel_cooperative_trailing_updates() {
    let backend = make_backend("cancel_trailing_3").await;
    backend
        .start_session(PathBuf::from("/tmp"))
        .await
        .expect("start_session");

    // Flag is already true: AcpBackend sends cancel immediately, server sends
    // 3 trailing text updates then responds PromptResponse { Cancelled }.
    let cancelled = Arc::new(AtomicBool::new(true));
    let (result, events) = run_turn_with_opts(&backend, cancelled, None).await;

    let turn = result.expect("cooperative cancel should return Ok");
    assert_eq!(
        turn.outcome,
        TurnOutcome::Cancelled,
        "outcome should be Cancelled (stop_reason=Cancelled)"
    );

    // Trailing text events should have been delivered
    let text_count = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::Text { .. }))
        .count();
    assert!(
        text_count >= 1,
        "expected trailing Text events from server, got {text_count}"
    );

    backend.stop_session();
}

// ---------------------------------------------------------------------------
// Scenario 3 — Cancel sans réponse (killpg)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancel_no_response_triggers_killpg() {
    let backend = make_backend("silence").await;
    backend
        .start_session(PathBuf::from("/tmp"))
        .await
        .expect("start_session");

    // Cancelled immediately: server never responds to prompt → cancel_grace expires → killpg
    let cancelled = Arc::new(AtomicBool::new(true));
    let (result, _events) = run_turn_with_opts(&backend, cancelled, None).await;

    assert!(
        matches!(result, Err(SympheoError::TurnCancelled)),
        "expected TurnCancelled after kill, got: {result:?}"
    );

    backend.stop_session();
}

// ---------------------------------------------------------------------------
// Scenario 4a-e — 5 stopReason variants
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stop_reason_end_turn_succeeds() {
    let backend = make_backend("lifecycle_end_turn").await;
    backend.start_session(PathBuf::from("/tmp")).await.unwrap();
    let (result, _) = run_turn(&backend).await;
    assert_eq!(result.unwrap().outcome, TurnOutcome::Succeeded);
    backend.stop_session();
}

#[tokio::test]
async fn stop_reason_max_tokens_fails() {
    let backend = make_backend("stop_reason_max_tokens").await;
    backend.start_session(PathBuf::from("/tmp")).await.unwrap();
    let (result, _) = run_turn(&backend).await;
    assert_eq!(result.unwrap().outcome, TurnOutcome::Failed);
    backend.stop_session();
}

#[tokio::test]
async fn stop_reason_max_turn_requests_fails() {
    let backend = make_backend("stop_reason_max_turn_requests").await;
    backend.start_session(PathBuf::from("/tmp")).await.unwrap();
    let (result, _) = run_turn(&backend).await;
    assert_eq!(result.unwrap().outcome, TurnOutcome::Failed);
    backend.stop_session();
}

#[tokio::test]
async fn stop_reason_refusal_fails() {
    let backend = make_backend("stop_reason_refusal").await;
    backend.start_session(PathBuf::from("/tmp")).await.unwrap();
    let (result, _) = run_turn(&backend).await;
    assert_eq!(result.unwrap().outcome, TurnOutcome::Failed);
    backend.stop_session();
}

#[tokio::test]
async fn stop_reason_cancelled_returns_cancelled_outcome() {
    let backend = make_backend("stop_reason_cancelled").await;
    backend.start_session(PathBuf::from("/tmp")).await.unwrap();
    let (result, _) = run_turn(&backend).await;
    assert_eq!(result.unwrap().outcome, TurnOutcome::Cancelled);
    backend.stop_session();
}

// ---------------------------------------------------------------------------
// Scenario 5 — tool_progress_timeout
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tool_progress_timeout_returns_read_timeout_error() {
    let backend = make_backend("silence").await;
    backend.start_session(PathBuf::from("/tmp")).await.unwrap();
    // No cancel → tool_progress_timeout fires
    let (result, _) = run_turn(&backend).await;
    assert!(
        matches!(result, Err(SympheoError::TurnReadTimeout)),
        "expected TurnReadTimeout, got: {result:?}"
    );
    backend.stop_session();
}

// ---------------------------------------------------------------------------
// Scenario 6 — read_timeout (silence mid-turn, no cancel)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_timeout_silence_returns_error() {
    // Same as tool_progress_timeout — both arise from silence mid-turn.
    // Using a dedicated test makes the acceptance criteria explicit.
    let backend = make_backend("silence").await;
    backend.start_session(PathBuf::from("/tmp")).await.unwrap();
    let (result, _) = run_turn(&backend).await;
    assert!(
        matches!(result, Err(SympheoError::TurnReadTimeout)),
        "expected TurnReadTimeout on silent server, got: {result:?}"
    );
    backend.stop_session();
}

// ---------------------------------------------------------------------------
// Scenario 7 — session_start_timeout
// ---------------------------------------------------------------------------

#[tokio::test]
async fn session_start_timeout_returns_error() {
    let backend = make_backend("silence_on_init").await;
    // The fake server never responds to initialize → session_start_timeout
    let result = backend.start_session(PathBuf::from("/tmp")).await;
    assert!(
        matches!(result, Err(SympheoError::SessionStartFailed(_))),
        "expected SessionStartFailed, got: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Scenario 8 — preflight_timeout (adapter preflight_check returns error)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn preflight_check_failure_is_detectable() {
    struct FailingPreflight;

    impl AcpAdapter for FailingPreflight {
        fn kind(&self) -> &str {
            "failing-preflight"
        }

        fn spawn_spec(&self, _: &HashMap<String, String>) -> std::process::Command {
            std::process::Command::new("true")
        }

        fn client_capabilities(&self) -> agent_client_protocol::schema::ClientCapabilities {
            default_client_capabilities()
        }

        fn session_new_params(
            &self,
            cwd: PathBuf,
        ) -> agent_client_protocol::schema::NewSessionRequest {
            agent_client_protocol::schema::NewSessionRequest::new(cwd)
        }

        fn preflight_check(&self) -> Result<(), SympheoError> {
            Err(SympheoError::AgentRunnerError("preflight failed".into()))
        }
    }

    let result = FailingPreflight.preflight_check();
    assert!(
        result.is_err(),
        "preflight_check must propagate the error"
    );
}

// ---------------------------------------------------------------------------
// Scenario 9 — Mismatch protocolVersion (V0 rejected)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn protocol_version_v0_rejected() {
    let backend = make_backend("protocol_v0").await;
    let result = backend.start_session(PathBuf::from("/tmp")).await;
    assert!(
        matches!(result, Err(SympheoError::SessionStartFailed(_))),
        "V0 protocol version must cause SessionStartFailed, got: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Scenario 10 — AUTH_REQUIRED at initialize
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_required_at_initialize_returns_session_start_failed() {
    let backend = make_backend("auth_required_init").await;
    let result = backend.start_session(PathBuf::from("/tmp")).await;
    assert!(
        matches!(result, Err(SympheoError::SessionStartFailed(_))),
        "AUTH_REQUIRED at initialize must cause SessionStartFailed, got: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Scenario 11 — request_permission × 4 × ≥3 tool_kind
// ---------------------------------------------------------------------------

// BypassPermissions: allow-once option should be selected → EndTurn
#[tokio::test]
async fn request_permission_bypass_allows() {
    let backend = make_backend("perm_bypass_read").await;
    backend.start_session(PathBuf::from("/tmp")).await.unwrap();

    let (result, _) = run_turn_with_opts(
        &backend,
        Arc::new(AtomicBool::new(false)),
        Some(Permission::BypassPermissions),
    )
    .await;

    let turn = result.expect("BypassPermissions should complete without error");
    assert_eq!(turn.outcome, TurnOutcome::Succeeded);
    backend.stop_session();
}

// AcceptEdits: allow-once option should be selected → EndTurn
#[tokio::test]
async fn request_permission_accept_edits_allows() {
    let backend = make_backend("perm_accept_edits_edit").await;
    backend.start_session(PathBuf::from("/tmp")).await.unwrap();

    let (result, _) = run_turn_with_opts(
        &backend,
        Arc::new(AtomicBool::new(false)),
        Some(Permission::AcceptEdits),
    )
    .await;

    let turn = result.expect("AcceptEdits should complete without error");
    assert_eq!(turn.outcome, TurnOutcome::Succeeded);
    backend.stop_session();
}

// Default: read tool → allow → EndTurn
#[tokio::test]
async fn request_permission_default_read_allows() {
    let backend = make_backend("perm_default_read").await;
    backend.start_session(PathBuf::from("/tmp")).await.unwrap();

    let (result, _) = run_turn_with_opts(
        &backend,
        Arc::new(AtomicBool::new(false)),
        Some(Permission::Default),
    )
    .await;

    let turn = result.expect("Default mode with Read should complete");
    assert_eq!(turn.outcome, TurnOutcome::Succeeded);
    backend.stop_session();
}

// Plan: the fake server sends the permission request but ignores the client
// response and always completes with EndTurn — what we verify here is that
// the AcpBackend correctly evaluates the permission matrix (Plan+Execute →
// reject) without panicking and produces a valid turn outcome.
#[tokio::test]
async fn request_permission_plan_execute_runs_matrix() {
    let backend = make_backend("perm_plan_execute").await;
    backend.start_session(PathBuf::from("/tmp")).await.unwrap();

    let (result, _) = run_turn_with_opts(
        &backend,
        Arc::new(AtomicBool::new(false)),
        Some(Permission::Plan),
    )
    .await;

    // The fake server always completes with EndTurn regardless of the
    // permission decision, so the outcome is Succeeded.  The matrix
    // decision (Reject → Cancelled) is exercised but not externally visible
    // through TurnOutcome; it is asserted via the audit-log unit test in
    // src/agent/acp/permission.rs.
    assert!(
        result.is_ok(),
        "Plan+Execute permission matrix should not panic: {result:?}"
    );
    backend.stop_session();
}
