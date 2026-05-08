use crate::orchestrator::state::{OrchestratorState, RunningEntry};
use axum::{
    Router,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{Html, Json},
    routing::{get, post},
};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;

pub type SharedState = Arc<RwLock<OrchestratorState>>;

pub async fn start_server(port: u16, state: SharedState) -> Result<(), crate::error::SympheoError> {
    let app = Router::new()
        .route("/", get(dashboard))
        .route("/api/v1/state", get(api_state))
        .route("/api/v1/refresh", post(api_refresh))
        .route("/api/v1/{issue_identifier}", get(api_issue))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port))
        .await
        .map_err(|e| crate::error::SympheoError::Io(e.to_string()))?;
    let actual_port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    tracing::info!(port = actual_port, "HTTP server listening");
    axum::serve(listener, app)
        .await
        .map_err(|e| crate::error::SympheoError::Io(e.to_string()))?;
    Ok(())
}

async fn dashboard(State(state): State<SharedState>) -> (StatusCode, Html<String>) {
    let st = state.read().await;
    let now = chrono::Utc::now();

    let running_count = st.running.len();
    let retrying_count = st.retry_attempts.len();
    let total_tokens = st.codex_totals.total_tokens;
    let runtime_secs = st.codex_totals.seconds_running as u64;

    let running_rows: String = st
        .running
        .values()
        .map(|e| {
            let sess = e.session.as_ref();
            let last_event = sess
                .and_then(|s| s.last_event.as_ref())
                .map(|s| s.as_str())
                .unwrap_or("-");
            let last_msg = sess
                .and_then(|s| s.last_message.as_ref())
                .map(|s| {
                    let mut txt = s.clone();
                    if txt.len() > 30 {
                        txt.truncate(30);
                        txt.push_str("...");
                    }
                    html_escape(&txt)
                })
                .unwrap_or_else(|| "-".to_string());
            let tokens = sess
                .map(|s| format!("{} / {}", s.input_tokens, s.output_tokens))
                .unwrap_or_else(|| "-".to_string());
            let started = e.started_at.format("%H:%M:%S").to_string();
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                html_escape(&e.issue.identifier),
                html_escape(&e.issue.state),
                sess.map(|s| html_escape(&s.session_id)).unwrap_or_else(|| "-".to_string()),
                e.turn_count,
                started,
                html_escape(last_event),
                last_msg,
                tokens
            )
        })
        .collect();

    let retry_rows: String = st
        .retry_attempts
        .values()
        .map(|r| {
            let error_text = r
                .error
                .as_ref()
                .map(|e| {
                    let mut txt = e.clone();
                    if txt.len() > 50 {
                        txt.truncate(50);
                        txt.push_str("...");
                    }
                    html_escape(&txt)
                })
                .unwrap_or_else(|| "-".to_string());
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{:.0}s</td></tr>",
                html_escape(&r.identifier),
                r.attempt,
                error_text,
                r.due_at
                    .saturating_duration_since(std::time::Instant::now())
                    .as_secs_f64()
            )
        })
        .collect();

    let last_tick = st
        .last_tick_at
        .map(|t| {
            let secs = (now - t).num_seconds();
            if secs < 60 {
                format!("{}s ago", secs)
            } else {
                format!("{}m ago", secs / 60)
            }
        })
        .unwrap_or_else(|| "never".to_string());

    let terminal_states = vec!["done".to_string(), "closed".to_string()];
    let mut state_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let mut recent_changes: Vec<&RunningEntry> = Vec::new();
    let mut blocked_entries: Vec<&RunningEntry> = Vec::new();

    for entry in st.running.values() {
        *state_counts.entry(entry.issue.state.clone()).or_insert(0) += 1;
        recent_changes.push(entry);
        if entry.issue.is_blocked(&terminal_states) {
            blocked_entries.push(entry);
        }
    }

    recent_changes.sort_by_key(|b| std::cmp::Reverse(b.last_state_change_at));

    let state_summary_cards: String = state_counts
        .iter()
        .map(|(state, count)| {
            format!(
                r#"<article><h5>{}</h5><p class="display">{}</p></article>"#,
                html_escape(state),
                count
            )
        })
        .collect();

    let recent_rows: String = recent_changes
        .iter()
        .take(10)
        .map(|e| {
            let secs = (now - e.last_state_change_at).num_seconds();
            let ago = if secs < 60 {
                format!("{}s ago", secs)
            } else {
                format!("{}m ago", secs / 60)
            };
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                html_escape(&e.issue.identifier),
                html_escape(&e.issue.state),
                ago
            )
        })
        .collect();

    let blocked_rows: String = blocked_entries
        .iter()
        .map(|e| {
            let blockers = e
                .issue
                .blocked_by
                .iter()
                .filter_map(|b| b.identifier.as_ref())
                .map(|id| html_escape(id))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "<tr><td>{}</td><td>{}</td><td>Blocked by: {}</td></tr>",
                html_escape(&e.issue.identifier),
                html_escape(&e.issue.state),
                if blockers.is_empty() {
                    "-".to_string()
                } else {
                    blockers
                }
            )
        })
        .collect();

    let delayed_rows: String = st
        .retry_attempts
        .values()
        .map(|r| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>Retry attempt #{}</td></tr>",
                html_escape(&r.identifier),
                "retrying",
                r.attempt
            )
        })
        .collect();

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en" data-theme="dark">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Sympheo Dashboard</title>
  <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/@picocss/pico@2/css/pico.min.css">
  <style>
    .status-dot {{ width: 10px; height: 10px; border-radius: 50%; display: inline-block; margin-right: 6px; }}
    .status-running {{ background: var(--pico-color-green-500); }}
    .status-retrying {{ background: var(--pico-color-amber-500); }}
    .status-error {{ background: var(--pico-color-red-500); }}
    .display {{ font-size: 2rem; font-weight: bold; margin: 0; }}
    .card article {{ margin-bottom: 0; }}
  </style>
</head>
<body>
  <main class="container">
    <h1>🔧 Sympheo Orchestrator</h1>
    <p style="font-size:0.85rem; color:var(--pico-muted-color);">Last tick: {}</p>
    <div class="grid">
      <article>
        <h3>Running</h3>
        <p class="display">{}</p>
      </article>
      <article>
        <h3>Retrying</h3>
        <p class="display">{}</p>
      </article>
      <article>
        <h3>Tokens</h3>
        <p class="display">{}</p>
      </article>
      <article>
        <h3>Runtime</h3>
        <p class="display">{}s</p>
      </article>
    </div>

    <h2>Ticket Summary</h2>
    <div class="grid">
      {}
    </div>

    <h3>Recent Movements</h3>
    <table>
      <thead>
        <tr><th>Issue</th><th>State</th><th>Last Changed</th></tr>
      </thead>
      <tbody>{}</tbody>
    </table>

    <h3>Blocked or Delayed</h3>
    <table>
      <thead>
        <tr><th>Issue</th><th>State</th><th>Reason</th></tr>
      </thead>
      <tbody>{}</tbody>
    </table>

    <h2>Active Sessions</h2>
    <table>
      <thead>
        <tr>
          <th>Issue</th>
          <th>State</th>
          <th>Session</th>
          <th>Turns</th>
          <th>Started</th>
          <th>Last Event</th>
          <th>Last Message</th>
          <th>Tokens (in/out)</th>
        </tr>
      </thead>
      <tbody>{}</tbody>
    </table>

    <h2>Retry Queue</h2>
    <table>
      <thead>
        <tr>
          <th>Issue</th>
          <th>Attempt</th>
          <th>Error</th>
          <th>Due In</th>
        </tr>
      </thead>
      <tbody>{}</tbody>
    </table>
  </main>
  <script>
    setInterval(() => location.reload(), 5000);
  </script>
</body>
</html>"#,
        last_tick,
        running_count,
        retrying_count,
        total_tokens,
        runtime_secs,
        if state_summary_cards.is_empty() {
            "<p>No active tickets</p>".to_string()
        } else {
            state_summary_cards
        },
        if recent_rows.is_empty() {
            "<tr><td colspan=3 style='text-align:center;'>No recent changes</td></tr>".to_string()
        } else {
            recent_rows
        },
        if blocked_rows.is_empty() && delayed_rows.is_empty() {
            "<tr><td colspan=3 style='text-align:center;'>No blocked or delayed tickets</td></tr>"
                .to_string()
        } else {
            blocked_rows + &delayed_rows
        },
        if running_rows.is_empty() {
            "<tr><td colspan=8 style='text-align:center;'>No active sessions</td></tr>".to_string()
        } else {
            running_rows
        },
        if retry_rows.is_empty() {
            "<tr><td colspan=4 style='text-align:center;'>No retries queued</td></tr>".to_string()
        } else {
            retry_rows
        }
    );
    (StatusCode::OK, Html(html))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

async fn api_state(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let st = state.read().await;
    let now = chrono::Utc::now().to_rfc3339();
    let running: Vec<serde_json::Value> = st
        .running
        .values()
        .map(|e| {
            let sess = e.session.as_ref();
            json!({
                "issue_id": e.issue.id,
                "issue_identifier": e.issue.identifier,
                "state": e.issue.state,
                "session_id": sess.map(|s| s.session_id.clone()).unwrap_or_default(),
                "turn_count": e.turn_count,
                "started_at": e.started_at.to_rfc3339(),
                "last_event": sess.and_then(|s| s.last_event.clone()),
                "last_message": sess.and_then(|s| s.last_message.clone()),
                "last_event_at": sess.and_then(|s| s.last_timestamp.map(|t| t.to_rfc3339())),
                "tokens": sess.map(|s| json!({
                    "input_tokens": s.input_tokens,
                    "output_tokens": s.output_tokens,
                    "total_tokens": s.total_tokens,
                })),
            })
        })
        .collect();
    let retrying: Vec<serde_json::Value> = st
        .retry_attempts
        .values()
        .map(|r| {
            json!({
                "issue_id": r.issue_id,
                "issue_identifier": r.identifier,
                "attempt": r.attempt,
                "error": r.error,
            })
        })
        .collect();
    let terminal_states = vec!["done".to_string(), "closed".to_string()];
    let mut state_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let mut recent_changes: Vec<&RunningEntry> = Vec::new();
    let mut blocked_entries: Vec<&RunningEntry> = Vec::new();
    for entry in st.running.values() {
        *state_counts.entry(entry.issue.state.clone()).or_insert(0) += 1;
        recent_changes.push(entry);
        if entry.issue.is_blocked(&terminal_states) {
            blocked_entries.push(entry);
        }
    }
    recent_changes.sort_by_key(|b| std::cmp::Reverse(b.last_state_change_at));
    let summary = json!({
        "by_state": state_counts,
        "recent_changes": recent_changes.iter().take(10).map(|e| json!({
            "identifier": e.issue.identifier,
            "state": e.issue.state,
            "last_state_change_at": e.last_state_change_at.to_rfc3339(),
        })).collect::<Vec<_>>(),
        "blocked": blocked_entries.iter().map(|e| json!({
            "identifier": e.issue.identifier,
            "state": e.issue.state,
            "blocked_by": e.issue.blocked_by.iter().filter_map(|b| b.identifier.clone()).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "delayed": st.retry_attempts.values().map(|r| json!({
            "identifier": r.identifier,
            "attempt": r.attempt,
            "error": r.error,
        })).collect::<Vec<_>>(),
    });
    Json(json!({
        "generated_at": now,
        "counts": {
            "running": st.running.len(),
            "retrying": st.retry_attempts.len()
        },
        "running": running,
        "retrying": retrying,
        "summary": summary,
        "codex_totals": {
            "input_tokens": st.codex_totals.input_tokens,
            "output_tokens": st.codex_totals.output_tokens,
            "total_tokens": st.codex_totals.total_tokens,
            "seconds_running": st.codex_totals.seconds_running,
        },
        "rate_limits": st.codex_rate_limits,
    }))
}

async fn api_refresh(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let notify = {
        let st = state.read().await;
        st.refresh_notify.clone()
    };
    notify.notify_one();
    Json(json!({
        "queued": true,
        "coalesced": false,
        "requested_at": chrono::Utc::now().to_rfc3339(),
        "operations": ["poll", "reconcile"]
    }))
}

async fn api_issue(
    State(state): State<SharedState>,
    AxumPath(issue_identifier): AxumPath<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let st = state.read().await;
    let entry = st
        .running
        .values()
        .find(|e| e.issue.identifier == issue_identifier);
    if let Some(entry) = entry {
        let sess = entry.session.as_ref();
        Ok(Json(json!({
            "issue_identifier": entry.issue.identifier,
            "issue_id": entry.issue.id,
            "status": "running",
            "started_at": entry.started_at.to_rfc3339(),
            "turn_count": entry.turn_count,
            "retry_attempt": entry.retry_attempt,
            "session": sess.map(|s| json!({
                "session_id": s.session_id,
                "thread_id": s.thread_id,
                "turn_id": s.turn_id,
                "last_event": s.last_event,
                "last_message": s.last_message,
                "last_timestamp": s.last_timestamp.map(|t| t.to_rfc3339()),
                "input_tokens": s.input_tokens,
                "output_tokens": s.output_tokens,
                "total_tokens": s.total_tokens,
                "turn_count": s.turn_count,
            })),
            "recent_events": sess.map(|s| {
                let mut ev = vec![];
                if let Some(ref e) = s.last_event {
                    ev.push(json!({"event": e, "at": s.last_timestamp.map(|t| t.to_rfc3339())}));
                }
                ev
            }).unwrap_or_default(),
        })))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::state::{OrchestratorState, RunningEntry};
    use crate::tracker::model::{Issue, LiveSession, RetryEntry, TokenTotals};

    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn test_html_escape() {
        assert_eq!(
            html_escape("<script>alert('xss')</script>"),
            "&lt;script&gt;alert('xss')&lt;/script&gt;"
        );
        assert_eq!(html_escape("foo & bar"), "foo &amp; bar");
        assert_eq!(html_escape("\"quoted\""), "&quot;quoted&quot;");
        assert_eq!(html_escape("<div>"), "&lt;div&gt;");
    }

    #[tokio::test]
    async fn test_dashboard_with_running_and_retries() {
        let mut state = OrchestratorState::new(5000, 5);
        state.codex_totals = TokenTotals {
            input_tokens: 100,
            output_tokens: 200,
            total_tokens: 300,
            seconds_running: 42.0,
        };
        state.last_tick_at = Some(chrono::Utc::now());

        let session = LiveSession {
            session_id: "sess-1".into(),
            thread_id: "thread-1".into(),
            turn_id: "turn-1".into(),
            agent_pid: None,
            last_event: Some("StepFinish".into()),
            last_message: Some("Build <complete>".into()),
            last_timestamp: Some(chrono::Utc::now()),
            input_tokens: 50,
            output_tokens: 100,
            total_tokens: 150,
            last_reported_input_tokens: 0,
            last_reported_output_tokens: 0,
            last_reported_total_tokens: 0,
            turn_count: 1,
            pr_url: None,
        };

        state.running.insert(
            "1".into(),
            RunningEntry {
                issue: Issue {
                    id: "1".into(),
                    identifier: "TEST-1".into(),
                    title: "a".into(),
                    description: None,
                    priority: None,
                    state: "in progress".into(),
                    branch_name: None,
                    url: None,
                    labels: vec![],
                    blocked_by: vec![],
                    ..Default::default()
                },
                session: Some(session),
                started_at: chrono::Utc::now(),
                retry_attempt: None,
                turn_count: 3,
                stagnation_counter: 0,
                last_state_change_at: chrono::Utc::now(),
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        );

        state.retry_attempts.insert(
            "2".into(),
            RetryEntry {
                issue_id: "2".into(),
                identifier: "TEST-2".into(),
                attempt: 2,
                error: Some("connection refused".into()),
                due_at: std::time::Instant::now() + std::time::Duration::from_secs(60),
            },
        );

        let shared = Arc::new(RwLock::new(state));
        let (status, Html(body)) = dashboard(State(shared)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("TEST-1"));
        assert!(body.contains("in progress"));
        assert!(body.contains("sess-1"));
        assert!(body.contains("StepFinish"));
        assert!(body.contains("Build &lt;complete&gt;"));
        assert!(body.contains("50 / 100"));
        assert!(body.contains("TEST-2"));
        assert!(body.contains("connection refused"));
        assert!(body.contains("Running"));
        assert!(body.contains("Retrying"));
        assert!(body.contains("Tokens"));
        assert!(body.contains("Runtime"));
    }

    #[tokio::test]
    async fn test_dashboard_empty() {
        let state = OrchestratorState::new(5000, 5);
        let shared = Arc::new(RwLock::new(state));
        let (status, Html(body)) = dashboard(State(shared)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("No active sessions"));
        assert!(body.contains("No retries queued"));
    }

    #[tokio::test]
    async fn test_dashboard_no_last_tick() {
        let mut state = OrchestratorState::new(5000, 5);
        state.last_tick_at = None;
        state.retry_attempts.insert(
            "1".into(),
            RetryEntry {
                issue_id: "1".into(),
                identifier: "TEST-1".into(),
                attempt: 1,
                error: None,
                due_at: std::time::Instant::now(),
            },
        );
        let shared = Arc::new(RwLock::new(state));
        let (status, Html(body)) = dashboard(State(shared)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("never"));
        assert!(body.contains("TEST-1"));
    }

    #[tokio::test]
    async fn test_dashboard_long_message_truncate() {
        let mut state = OrchestratorState::new(5000, 5);
        state.running.insert(
            "1".into(),
            RunningEntry {
                issue: Issue {
                    id: "1".into(),
                    identifier: "TEST-1".into(),
                    title: "a".into(),
                    description: None,
                    priority: None,
                    state: "todo".into(),
                    branch_name: None,
                    url: None,
                    labels: vec![],
                    blocked_by: vec![],
                    ..Default::default()
                },
                session: Some(LiveSession {
                    session_id: "s1".into(),
                    thread_id: "t1".into(),
                    turn_id: "u1".into(),
                    agent_pid: None,
                    last_event: Some("Text".into()),
                    last_message: Some("a".repeat(100)),
                    last_timestamp: Some(chrono::Utc::now()),
                    input_tokens: 0,
                    output_tokens: 0,
                    total_tokens: 0,
                    last_reported_input_tokens: 0,
                    last_reported_output_tokens: 0,
                    last_reported_total_tokens: 0,
                    turn_count: 0,
                    pr_url: None,
                }),
                started_at: chrono::Utc::now(),
                retry_attempt: None,
                turn_count: 0,
                stagnation_counter: 0,
                last_state_change_at: chrono::Utc::now(),
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        );
        let shared = Arc::new(RwLock::new(state));
        let (status, Html(body)) = dashboard(State(shared)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("..."));
    }

    #[tokio::test]
    async fn test_dashboard_retry_long_error_and_old_tick() {
        let mut state = OrchestratorState::new(5000, 5);
        state.last_tick_at = Some(chrono::Utc::now() - chrono::Duration::seconds(120));
        state.retry_attempts.insert(
            "1".into(),
            RetryEntry {
                issue_id: "1".into(),
                identifier: "TEST-1".into(),
                attempt: 1,
                error: Some("a".repeat(100)),
                due_at: std::time::Instant::now(),
            },
        );
        let shared = Arc::new(RwLock::new(state));
        let (status, Html(body)) = dashboard(State(shared)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("..."));
        assert!(body.contains("2m ago"));
    }

    #[tokio::test]
    async fn test_api_issue_with_session_data() {
        let mut state = OrchestratorState::new(5000, 5);
        let ts = chrono::Utc::now();
        state.running.insert(
            "1".into(),
            RunningEntry {
                issue: Issue {
                    id: "1".into(),
                    identifier: "TEST-1".into(),
                    title: "a".into(),
                    description: None,
                    priority: None,
                    state: "todo".into(),
                    branch_name: None,
                    url: None,
                    labels: vec![],
                    blocked_by: vec![],
                    ..Default::default()
                },
                session: Some(LiveSession {
                    session_id: "s1".into(),
                    thread_id: "t1".into(),
                    turn_id: "u1".into(),
                    agent_pid: None,
                    last_event: Some("StepFinish".into()),
                    last_message: Some("done".into()),
                    last_timestamp: Some(ts),
                    input_tokens: 10,
                    output_tokens: 20,
                    total_tokens: 30,
                    last_reported_input_tokens: 0,
                    last_reported_output_tokens: 0,
                    last_reported_total_tokens: 0,
                    turn_count: 1,
                    pr_url: None,
                }),
                started_at: ts,
                retry_attempt: None,
                turn_count: 1,
                stagnation_counter: 0,
                last_state_change_at: ts,
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        );
        let shared = Arc::new(RwLock::new(state));
        let result = api_issue(State(shared), AxumPath("TEST-1".into())).await;
        assert!(result.is_ok());
        let json = result.unwrap().0;
        assert_eq!(json["issue_identifier"], "TEST-1");
        assert!(json["session"].is_object());
        assert!(!json["recent_events"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_dashboard_summary_sections() {
        let mut state = OrchestratorState::new(5000, 5);
        let ts = chrono::Utc::now();
        state.running.insert(
            "1".into(),
            RunningEntry {
                issue: Issue {
                    id: "1".into(),
                    identifier: "TEST-1".into(),
                    title: "a".into(),
                    description: None,
                    priority: None,
                    state: "todo".into(),
                    branch_name: None,
                    url: None,
                    labels: vec![],
                    blocked_by: vec![],
                    ..Default::default()
                },
                session: None,
                started_at: ts,
                retry_attempt: None,
                turn_count: 0,
                stagnation_counter: 0,
                last_state_change_at: ts,
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        );
        state.running.insert(
            "2".into(),
            RunningEntry {
                issue: Issue {
                    id: "2".into(),
                    identifier: "TEST-2".into(),
                    title: "b".into(),
                    description: None,
                    priority: None,
                    state: "in progress".into(),
                    branch_name: None,
                    url: None,
                    labels: vec![],
                    blocked_by: vec![crate::tracker::model::BlockerRef {
                        id: Some("3".into()),
                        identifier: Some("TEST-3".into()),
                        state: Some("in progress".into()),
                    }],
                    ..Default::default()
                },
                session: None,
                started_at: ts,
                retry_attempt: None,
                turn_count: 0,
                stagnation_counter: 0,
                last_state_change_at: ts + chrono::Duration::seconds(1),
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        );
        state.retry_attempts.insert(
            "3".into(),
            RetryEntry {
                issue_id: "3".into(),
                identifier: "TEST-3".into(),
                attempt: 1,
                error: Some("timeout".into()),
                due_at: std::time::Instant::now(),
            },
        );

        let shared = Arc::new(RwLock::new(state));
        let (status, Html(body)) = dashboard(State(shared)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Ticket Summary"));
        assert!(body.contains("Recent Movements"));
        assert!(body.contains("Blocked or Delayed"));
        assert!(body.contains("todo") && body.contains("in progress"));
        assert!(body.contains("TEST-2"));
        assert!(body.contains("Blocked by: TEST-3"));
        assert!(body.contains("Retry attempt #1"));
    }

    #[tokio::test]
    async fn test_api_state_summary() {
        let mut state = OrchestratorState::new(5000, 5);
        let ts = chrono::Utc::now();
        state.running.insert(
            "1".into(),
            RunningEntry {
                issue: Issue {
                    id: "1".into(),
                    identifier: "TEST-1".into(),
                    title: "a".into(),
                    description: None,
                    priority: None,
                    state: "todo".into(),
                    branch_name: None,
                    url: None,
                    labels: vec![],
                    blocked_by: vec![crate::tracker::model::BlockerRef {
                        id: Some("2".into()),
                        identifier: Some("TEST-2".into()),
                        state: Some("in progress".into()),
                    }],
                    ..Default::default()
                },
                session: None,
                started_at: ts,
                retry_attempt: None,
                turn_count: 0,
                stagnation_counter: 0,
                last_state_change_at: ts,
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        );
        let shared = Arc::new(RwLock::new(state));
        let json = api_state(State(shared)).await.0;
        assert!(json["summary"].is_object());
        assert_eq!(json["summary"]["by_state"]["todo"], 1);
        assert_eq!(
            json["summary"]["recent_changes"].as_array().unwrap().len(),
            1
        );
        assert_eq!(json["summary"]["blocked"].as_array().unwrap().len(), 1);
        assert_eq!(json["summary"]["blocked"][0]["identifier"], "TEST-1");
        assert_eq!(json["summary"]["delayed"].as_array().unwrap().len(), 0);
    }
}
