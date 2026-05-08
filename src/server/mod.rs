use crate::orchestrator::state::OrchestratorState;
use axum::{
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
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

async fn dashboard(State(state): State<SharedState>) -> (StatusCode, String) {
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
                r.due_at.saturating_duration_since(std::time::Instant::now()).as_secs_f64()
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
        last_tick, running_count, retrying_count, total_tokens, runtime_secs,
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
    (StatusCode::OK, html)
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
    Json(json!({
        "generated_at": now,
        "counts": {
            "running": st.running.len(),
            "retrying": st.retry_attempts.len()
        },
        "running": running,
        "retrying": retrying,
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
