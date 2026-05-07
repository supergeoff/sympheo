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

pub async fn start_server(port: u16, state: SharedState) -> Result<(), crate::error::SymphonyError> {
    let app = Router::new()
        .route("/", get(dashboard))
        .route("/api/v1/state", get(api_state))
        .route("/api/v1/refresh", post(api_refresh))
        .route("/api/v1/:issue_identifier", get(api_issue))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port))
        .await
        .map_err(|e| crate::error::SymphonyError::Io(e.to_string()))?;
    let actual_port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    tracing::info!(port = actual_port, "HTTP server listening");
    axum::serve(listener, app)
        .await
        .map_err(|e| crate::error::SymphonyError::Io(e.to_string()))?;
    Ok(())
}

async fn dashboard(State(state): State<SharedState>) -> (StatusCode, String) {
    let st = state.read().await;
    let html = format!(
        "<html><body><h1>Symphony</h1><p>Running: {}</p><p>Retrying: {}</p></body></html>",
        st.running.len(),
        st.retry_attempts.len()
    );
    (StatusCode::OK, html)
}

async fn api_state(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let st = state.read().await;
    let now = chrono::Utc::now().to_rfc3339();
    let running: Vec<serde_json::Value> = st
        .running
        .values()
        .map(|e| {
            json!({
                "issue_id": e.issue.id,
                "issue_identifier": e.issue.identifier,
                "state": e.issue.state,
                "session_id": e.session.as_ref().map(|s| s.session_id.clone()).unwrap_or_default(),
                "turn_count": e.turn_count,
                "started_at": e.started_at.to_rfc3339(),
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

async fn api_refresh(State(_state): State<SharedState>) -> Json<serde_json::Value> {
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
        Ok(Json(json!({
            "issue_identifier": entry.issue.identifier,
            "issue_id": entry.issue.id,
            "status": "running",
            "started_at": entry.started_at.to_rfc3339(),
            "turn_count": entry.turn_count,
        })))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}
