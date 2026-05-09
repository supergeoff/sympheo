use crate::orchestrator::state::{OrchestratorState, RunningEntry};
use axum::{
    Router,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::{delete, get, post},
};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::RwLock;

pub type SharedState = Arc<RwLock<OrchestratorState>>;

pub async fn start_server(port: u16, state: SharedState) -> Result<(), crate::error::SympheoError> {
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port))
        .await
        .map_err(|e| crate::error::SympheoError::Io(e.to_string()))?;
    start_server_with_listener(listener, state).await
}

// Tests pre-bind the listener to avoid the find_free_port → start_server race
// where the kernel hands the same ephemeral port to two parallel test cases
// between port discovery and server bind.
pub async fn start_server_with_listener(
    listener: tokio::net::TcpListener,
    state: SharedState,
) -> Result<(), crate::error::SympheoError> {
    let app = Router::new()
        .route("/", get(dashboard))
        .route("/api/v1/state", get(api_state))
        .route("/api/v1/refresh", post(api_refresh))
        .route("/api/v1/{issue_identifier}", get(api_issue))
        .route("/api/v1/{issue_identifier}/cancel", post(api_cancel))
        .route("/api/v1/retry/{issue_identifier}", delete(api_retry_delete))
        // SPEC §13.7.2: unsupported methods on defined routes return 405 with
        // the JSON error envelope. Without this, axum returns an empty 405
        // body, which violates the documented error contract.
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(state);

    let actual_port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    tracing::info!(port = actual_port, "HTTP server listening");
    axum::serve(listener, app)
        .await
        .map_err(|e| crate::error::SympheoError::Io(e.to_string()))?;
    Ok(())
}

/// SPEC §13.7.2 error envelope: `{"error": {"code": ..., "message": ...}}`.
/// Used by every error path so clients have a single shape to parse.
fn json_error(
    status: StatusCode,
    code: &str,
    message: impl Into<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        status,
        Json(json!({
            "error": {
                "code": code,
                "message": message.into(),
            }
        })),
    )
}

async fn method_not_allowed() -> impl IntoResponse {
    json_error(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "Method not allowed for this route",
    )
}

async fn dashboard(State(state): State<SharedState>) -> (StatusCode, Html<String>) {
    let st = state.read().await;
    let now = chrono::Utc::now();

    let running_count = st.running.len();
    let retrying_count = st.retry_attempts.len();
    let total_tokens = st.cli_totals.total_tokens;
    let live_running_secs: i64 = st
        .running
        .values()
        .map(|e| (now - e.started_at).num_seconds().max(0))
        .sum();
    let runtime_secs = (st.cli_totals.seconds_running as i64 + live_running_secs).max(0) as u64;

    let running_rows: String = st
        .running
        .values()
        .map(|e| {
            let sess = e.session.as_ref();
            let last_event = sess
                .and_then(|s| s.last_event.as_ref())
                .map(|s| s.as_str())
                .unwrap_or("-");
            // SPEC §13.7: render the FULL last_message (no truncation). The
            // operator needs the complete content to spot a stuck or
            // misbehaving turn quickly. Wrap in a <details> for long messages
            // so the table stays readable.
            let last_msg = sess
                .and_then(|s| s.last_message.as_ref())
                .map(|s| render_last_message(s))
                .unwrap_or_else(|| "-".to_string());
            let tokens = sess
                .map(|s| format!("{} / {}", s.input_tokens, s.output_tokens))
                .unwrap_or_else(|| "-".to_string());
            let started = e.started_at.format("%H:%M:%S").to_string();
            // Operator kill switch (P6): HTMX-style POST, no page reload required.
            // The button calls POST /api/v1/<identifier>/cancel; the orchestrator's
            // watchdog will SIGKILL the subprocess group within ~1s and the next
            // tick converts the exit to a retry per §7.3 / §8.4.
            let identifier_url_safe = url_encode(&e.issue.identifier);
            let kill_button = format!(
                r#"<button class="contrast outline" style="padding:0.2rem 0.5rem; font-size:0.8rem;" hx-post="/api/v1/{}/cancel" hx-confirm="Kill worker {}? Will be retried after backoff." hx-swap="none">Kill</button>"#,
                identifier_url_safe,
                html_escape(&e.issue.identifier)
            );
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                html_escape(&e.issue.identifier),
                html_escape(&e.issue.state),
                sess.map(|s| html_escape(&s.session_id)).unwrap_or_else(|| "-".to_string()),
                e.turn_count,
                started,
                html_escape(last_event),
                last_msg,
                tokens,
                kill_button
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
  <script src="https://unpkg.com/htmx.org@2.0.4" integrity="sha384-HGfztofotfshcF7+8n44JQL2oJmowVChPTg48S+jvZoztPfvwD79OC/LTtG6dMp+" crossorigin="anonymous"></script>
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
          <th>Actions</th>
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
            "<tr><td colspan=9 style='text-align:center;'>No active sessions</td></tr>".to_string()
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

/// Minimal percent-encoding for path segments. Encodes characters that have
/// special meaning in URL paths (`#`, ` `, `?`, `/`, etc.). For sympheo
/// identifiers like `repo#42` this yields `repo%2342`, which `axum` decodes
/// transparently on the receiving end.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' {
            out.push(c);
        } else {
            let mut buf = [0u8; 4];
            for byte in c.encode_utf8(&mut buf).as_bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    out
}

/// Render a CLI last_message in the dashboard cell. SPEC §13.7 + CONTEXT.md
/// requires the FULL message be available to the operator (no silent
/// truncation). Short messages are rendered inline; long ones collapse into
/// a `<details>` so the table stays scannable while preserving full content.
fn render_last_message(msg: &str) -> String {
    let escaped = html_escape(msg);
    if msg.chars().count() <= 80 {
        return escaped;
    }
    let preview: String = msg.chars().take(60).collect();
    format!(
        "<details><summary>{}…</summary><pre style=\"white-space:pre-wrap; max-width:50ch;\">{}</pre></details>",
        html_escape(&preview),
        escaped
    )
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
    // SPEC §13.7.2 requires `due_at` on retrying[] rows. RetryEntry stores
    // due_at as a monotonic Instant; convert to wall-clock UTC by anchoring
    // against `now` (the conversion is approximate but accurate within one
    // tick of the Instant clock, which is fine for an observability surface).
    let now_inst = std::time::Instant::now();
    let now_utc_wall = chrono::Utc::now();
    let instant_to_utc = |t: std::time::Instant| -> chrono::DateTime<chrono::Utc> {
        if t >= now_inst {
            now_utc_wall
                + chrono::Duration::from_std(t - now_inst).unwrap_or(chrono::Duration::zero())
        } else {
            now_utc_wall
                - chrono::Duration::from_std(now_inst - t).unwrap_or(chrono::Duration::zero())
        }
    };
    let retrying: Vec<serde_json::Value> = st
        .retry_attempts
        .values()
        .map(|r| {
            json!({
                "issue_id": r.issue_id,
                "issue_identifier": r.identifier,
                "attempt": r.attempt,
                "due_at": instant_to_utc(r.due_at).to_rfc3339(),
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
        // SPEC §13.7.2: aggregate token/runtime totals are named `agent_totals`.
        "agent_totals": {
            "input_tokens": st.cli_totals.input_tokens,
            "output_tokens": st.cli_totals.output_tokens,
            "total_tokens": st.cli_totals.total_tokens,
            "seconds_running": st.cli_totals.seconds_running,
        },
        "rate_limits": st.cli_rate_limits,
    }))
}

async fn api_refresh(State(state): State<SharedState>) -> (StatusCode, Json<serde_json::Value>) {
    let notify = {
        let st = state.read().await;
        st.refresh_notify.clone()
    };
    notify.notify_one();
    // SPEC §13.7.2: refresh is a queued/best-effort trigger and SHOULD return
    // 202 Accepted to convey that the work happens asynchronously.
    (
        StatusCode::ACCEPTED,
        Json(json!({
            "queued": true,
            "coalesced": false,
            "requested_at": chrono::Utc::now().to_rfc3339(),
            "operations": ["poll", "reconcile"]
        })),
    )
}

/// SPEC §13.7 extension: operational control endpoint to cancel a running worker.
/// Sets `RunningEntry.cancelled` so the watchdog (`src/agent/backend/local.rs`)
/// kills the subprocess group within 1s. The orchestrator detects the worker
/// exit and converts it to a retry per §7.3 / §8.4.
///
/// Returns 200 with a JSON status payload on success, 404 if no running entry
/// matches the identifier.
async fn api_cancel(
    State(state): State<SharedState>,
    AxumPath(issue_identifier): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let st = state.read().await;
    let entry = st
        .running
        .values()
        .find(|e| e.issue.identifier == issue_identifier);
    match entry {
        Some(entry) => {
            entry.cancelled.store(true, Ordering::Relaxed);
            tracing::info!(
                issue_identifier = %issue_identifier,
                issue_id = %entry.issue.id,
                "operator-issued cancel via /api/v1/<id>/cancel"
            );
            Ok(Json(json!({
                "cancelled": true,
                "issue_identifier": issue_identifier,
                "issue_id": entry.issue.id,
                "requested_at": chrono::Utc::now().to_rfc3339(),
            })))
        }
        None => Err(json_error(
            StatusCode::NOT_FOUND,
            "issue_not_found",
            format!("No running issue with identifier {issue_identifier:?}"),
        )),
    }
}

/// SPEC §13.7 extension: operational control endpoint to remove an issue
/// from the retry queue (releases the claim so the issue stops being scheduled).
async fn api_retry_delete(
    State(state): State<SharedState>,
    AxumPath(issue_identifier): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let mut st = state.write().await;
    // Find the entry by identifier (retry entries store identifier alongside id).
    let issue_id = st
        .retry_attempts
        .values()
        .find(|r| r.identifier == issue_identifier)
        .map(|r| r.issue_id.clone());
    match issue_id {
        Some(id) => {
            st.retry_attempts.remove(&id);
            st.claimed.remove(&id);
            tracing::info!(
                issue_identifier = %issue_identifier,
                issue_id = %id,
                "operator-issued retry removal via DELETE /api/v1/retry/<id>"
            );
            Ok(Json(json!({
                "removed": true,
                "issue_identifier": issue_identifier,
                "issue_id": id,
                "requested_at": chrono::Utc::now().to_rfc3339(),
            })))
        }
        None => Err(json_error(
            StatusCode::NOT_FOUND,
            "issue_not_found",
            format!("No retry entry for identifier {issue_identifier:?}"),
        )),
    }
}

async fn api_issue(
    State(state): State<SharedState>,
    AxumPath(issue_identifier): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let st = state.read().await;
    let entry = st
        .running
        .values()
        .find(|e| e.issue.identifier == issue_identifier);
    if let Some(entry) = entry {
        let sess = entry.session.as_ref();
        // SPEC §13.7.2 nests: attempts.{restart_count, current_retry_attempt}
        // and running.tokens.{input_tokens, output_tokens, total_tokens}.
        // We add our extension fields under the running block (thread_id,
        // turn_id) which is forward-compatible per §13.7.
        let attempts = json!({
            "restart_count": 0,
            "current_retry_attempt": entry.retry_attempt.unwrap_or(0),
        });
        let running_block = sess.map(|s| {
            json!({
                "session_id": s.session_id,
                "turn_count": entry.turn_count,
                "state": entry.issue.state,
                "started_at": entry.started_at.to_rfc3339(),
                "last_event": s.last_event,
                "last_message": s.last_message,
                "last_event_at": s.last_timestamp.map(|t| t.to_rfc3339()),
                "tokens": {
                    "input_tokens": s.input_tokens,
                    "output_tokens": s.output_tokens,
                    "total_tokens": s.total_tokens,
                },
                "thread_id": s.thread_id,
                "turn_id": s.turn_id,
            })
        });
        let recent_events: Vec<serde_json::Value> = sess
            .and_then(|s| {
                s.last_event.as_ref().map(|e| {
                    vec![json!({
                        "at": s.last_timestamp.map(|t| t.to_rfc3339()),
                        "event": e,
                        "message": s.last_message,
                    })]
                })
            })
            .unwrap_or_default();
        Ok(Json(json!({
            "issue_identifier": entry.issue.identifier,
            "issue_id": entry.issue.id,
            "status": "running",
            "started_at": entry.started_at.to_rfc3339(),
            "attempts": attempts,
            "running": running_block,
            "retry": serde_json::Value::Null,
            "recent_events": recent_events,
            "last_error": serde_json::Value::Null,
            "tracked": {},
        })))
    } else {
        Err(json_error(
            StatusCode::NOT_FOUND,
            "issue_not_found",
            format!("No running issue with identifier {issue_identifier:?}"),
        ))
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

    #[test]
    fn test_url_encode_passthrough() {
        assert_eq!(url_encode("ABC-123"), "ABC-123");
        assert_eq!(url_encode("v1.2.3"), "v1.2.3");
    }

    #[test]
    fn test_url_encode_hash_and_slash() {
        // SPEC §13.7.2: GitHub identifiers contain '#' and need URL-encoding
        // when used as path segments.
        assert_eq!(url_encode("repo#42"), "repo%2342");
        assert_eq!(url_encode("a/b c"), "a%2Fb%20c");
    }

    #[test]
    fn test_render_last_message_short() {
        let msg = "Hello, world!";
        let rendered = render_last_message(msg);
        assert_eq!(rendered, "Hello, world!");
        assert!(!rendered.contains("<details>"));
    }

    #[test]
    fn test_render_last_message_long_uses_details() {
        let msg = "a".repeat(150);
        let rendered = render_last_message(&msg);
        assert!(rendered.contains("<details>"));
        assert!(rendered.contains(&msg));
    }

    #[test]
    fn test_render_last_message_escapes_html() {
        let rendered = render_last_message("<b>bold</b>");
        assert!(rendered.contains("&lt;b&gt;"));
        assert!(!rendered.contains("<b>"));
    }

    #[tokio::test]
    async fn test_api_cancel_sets_atomic() {
        let mut state = OrchestratorState::new(5000, 5);
        let cancelled_flag = Arc::new(AtomicBool::new(false));
        state.running.insert(
            "1".into(),
            RunningEntry {
                issue: Issue {
                    id: "1".into(),
                    identifier: "repo#42".into(),
                    title: "t".into(),
                    description: None,
                    priority: None,
                    state: "in progress".into(),
                    branch_name: None,
                    url: None,
                    labels: vec![],
                    blocked_by: vec![],
                    ..Default::default()
                },
                session: None,
                started_at: chrono::Utc::now(),
                retry_attempt: None,
                turn_count: 0,
                stagnation_counter: 0,
                last_state_change_at: chrono::Utc::now(),
                cancelled: cancelled_flag.clone(),
            },
        );
        let shared = Arc::new(RwLock::new(state));
        let result = api_cancel(State(shared.clone()), AxumPath("repo#42".into())).await;
        assert!(result.is_ok());
        assert!(cancelled_flag.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn test_api_cancel_unknown_returns_404() {
        let state = OrchestratorState::new(5000, 5);
        let shared = Arc::new(RwLock::new(state));
        let result = api_cancel(State(shared), AxumPath("nope#1".into())).await;
        let (status, Json(body)) = result.unwrap_err();
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "issue_not_found");
        assert!(body["error"]["message"].as_str().is_some());
    }

    #[tokio::test]
    async fn test_api_retry_delete_removes_entry() {
        let mut state = OrchestratorState::new(5000, 5);
        state.claimed.insert("1".into());
        state.retry_attempts.insert(
            "1".into(),
            RetryEntry {
                issue_id: "1".into(),
                identifier: "repo#42".into(),
                attempt: 2,
                error: Some("transient".into()),
                due_at: std::time::Instant::now(),
            },
        );
        let shared = Arc::new(RwLock::new(state));
        let result = api_retry_delete(State(shared.clone()), AxumPath("repo#42".into())).await;
        assert!(result.is_ok());
        let st = shared.read().await;
        assert!(!st.retry_attempts.contains_key("1"));
        assert!(!st.claimed.contains("1"));
    }

    #[tokio::test]
    async fn test_api_retry_delete_unknown_returns_404() {
        let state = OrchestratorState::new(5000, 5);
        let shared = Arc::new(RwLock::new(state));
        let result = api_retry_delete(State(shared), AxumPath("nope#1".into())).await;
        let (status, Json(body)) = result.unwrap_err();
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "issue_not_found");
        assert!(body["error"]["message"].as_str().is_some());
    }

    #[tokio::test]
    async fn test_dashboard_with_running_and_retries() {
        let mut state = OrchestratorState::new(5000, 5);
        state.cli_totals = TokenTotals {
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
    async fn test_dashboard_long_message_renders_full_in_details() {
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
        // P6: long messages render in a <details> with the FULL content
        // available (no truncation). The summary uses an ellipsis ('…').
        assert!(
            body.contains("<details>"),
            "long message should be wrapped in <details> for operator visibility"
        );
        assert!(
            body.contains(&"a".repeat(100)),
            "full message body should be present (no silent truncation)"
        );
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
        // SPEC §13.7.2: nested `running` object replaces the flat `session` block.
        let running = json["running"].as_object().expect("running block");
        assert_eq!(running["session_id"], "s1");
        assert_eq!(running["state"], "todo");
        assert_eq!(running["tokens"]["input_tokens"], 10);
        assert_eq!(running["tokens"]["output_tokens"], 20);
        assert_eq!(running["tokens"]["total_tokens"], 30);
        let attempts = json["attempts"].as_object().expect("attempts block");
        assert_eq!(attempts["restart_count"], 0);
        assert_eq!(attempts["current_retry_attempt"], 0);
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
