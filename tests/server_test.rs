use std::sync::Arc;
use tokio::sync::RwLock;

async fn bind_test_server(
    state: Arc<RwLock<sympheo::orchestrator::state::OrchestratorState>>,
) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        sympheo::server::start_server_with_listener(listener, state)
            .await
            .unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    port
}

#[tokio::test]
async fn test_server_dashboard() {
    let state = Arc::new(RwLock::new(
        sympheo::orchestrator::state::OrchestratorState::new(30000, 10),
    ));
    let port = bind_test_server(state).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{}/", port))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("Sympheo Orchestrator"));
    assert!(body.contains("Running"));
    assert!(body.contains("Retrying"));
    assert!(body.contains("picocss"));
    assert!(body.contains("setInterval"));
}

#[tokio::test]
async fn test_server_api_state() {
    let state = Arc::new(RwLock::new(
        sympheo::orchestrator::state::OrchestratorState::new(30000, 10),
    ));
    let port = bind_test_server(state).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{}/api/v1/state", port))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["counts"]["running"], 0);
    assert_eq!(body["counts"]["retrying"], 0);
    assert!(body["generated_at"].as_str().is_some());
}

#[tokio::test]
async fn test_server_api_state_with_data() {
    let mut orch_state = sympheo::orchestrator::state::OrchestratorState::new(30000, 10);
    let issue = sympheo::tracker::model::Issue {
        id: "1".into(),
        identifier: "TEST-1".into(),
        title: "Test".into(),
        description: None,
        priority: None,
        state: "todo".into(),
        branch_name: None,
        url: None,
        labels: vec![],
        blocked_by: vec![],
        ..Default::default()
    };
    let running_entry = sympheo::orchestrator::state::RunningEntry {
        issue: issue.clone(),
        session: Some(sympheo::tracker::model::LiveSession {
            session_id: "sess-1".into(),
            thread_id: "thread-1".into(),
            turn_id: "turn-1".into(),
            agent_pid: Some(1234),
            last_event: Some("event".into()),
            last_timestamp: Some(chrono::Utc::now()),
            last_message: Some("msg".into()),
            input_tokens: 100,
            output_tokens: 50,
            total_tokens: 150,
            last_reported_input_tokens: 100,
            last_reported_output_tokens: 50,
            last_reported_total_tokens: 150,
            turn_count: 2,
            pr_url: None,
        }),
        started_at: chrono::Utc::now(),
        retry_attempt: Some(1),
        turn_count: 2,
        cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        stagnation_counter: 0,
        last_state_change_at: chrono::Utc::now(),
    };
    orch_state.running.insert("1".into(), running_entry);
    orch_state.retry_attempts.insert(
        "1".into(),
        sympheo::tracker::model::RetryEntry {
            issue_id: "1".into(),
            identifier: "TEST-1".into(),
            attempt: 2,
            due_at: std::time::Instant::now(),
            error: Some("retry err".into()),
        },
    );
    let state = Arc::new(RwLock::new(orch_state));
    let port = bind_test_server(state).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{}/api/v1/state", port))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["counts"]["running"], 1);
    assert_eq!(body["counts"]["retrying"], 1);
    let running = body["running"].as_array().unwrap();
    assert_eq!(running[0]["session_id"], "sess-1");
    assert_eq!(running[0]["turn_count"], 2);
    assert_eq!(running[0]["last_event"], "event");
    assert_eq!(running[0]["last_message"], "msg");
    assert!(running[0]["last_event_at"].as_str().is_some());
    assert_eq!(running[0]["tokens"]["input_tokens"], 100);
    assert_eq!(running[0]["tokens"]["output_tokens"], 50);
    let retrying = body["retrying"].as_array().unwrap();
    assert_eq!(retrying[0]["error"], "retry err");
    assert!(body["summary"].is_object());
    assert_eq!(body["summary"]["by_state"]["todo"], 1);
    assert_eq!(
        body["summary"]["recent_changes"].as_array().unwrap().len(),
        1
    );
    assert_eq!(body["summary"]["blocked"].as_array().unwrap().len(), 0);
    assert_eq!(body["summary"]["delayed"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_server_api_refresh() {
    let state = Arc::new(RwLock::new(
        sympheo::orchestrator::state::OrchestratorState::new(30000, 10),
    ));
    let port = bind_test_server(state).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{}/api/v1/refresh", port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["queued"], true);
    assert!(body["requested_at"].as_str().is_some());
}

#[tokio::test]
async fn test_server_api_issue_found() {
    let mut orch_state = sympheo::orchestrator::state::OrchestratorState::new(30000, 10);
    orch_state.running.insert(
        "1".into(),
        sympheo::orchestrator::state::RunningEntry {
            issue: sympheo::tracker::model::Issue {
                id: "1".into(),
                identifier: "TEST-1".into(),
                title: "Test".into(),
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
            started_at: chrono::Utc::now(),
            retry_attempt: None,
            turn_count: 3,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            stagnation_counter: 0,
            last_state_change_at: chrono::Utc::now(),
        },
    );
    let state = Arc::new(RwLock::new(orch_state));
    let port = bind_test_server(state).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{}/api/v1/TEST-1", port))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["issue_identifier"], "TEST-1");
    assert_eq!(body["status"], "running");
    assert_eq!(body["turn_count"], 3);
    assert_eq!(body["retry_attempt"], serde_json::Value::Null);
}

#[tokio::test]
async fn test_server_api_refresh_triggers_notify() {
    let orch_state = sympheo::orchestrator::state::OrchestratorState::new(30000, 10);
    let notify = orch_state.refresh_notify.clone();
    let state = Arc::new(RwLock::new(orch_state));
    let port = bind_test_server(state).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{}/api/v1/refresh", port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let timeout = tokio::time::Duration::from_secs(2);
    let notified = tokio::time::timeout(timeout, notify.notified()).await;
    assert!(
        notified.is_ok(),
        "refresh_notify was not triggered within 2s"
    );
}

#[tokio::test]
async fn test_server_api_issue_not_found() {
    let state = Arc::new(RwLock::new(
        sympheo::orchestrator::state::OrchestratorState::new(30000, 10),
    ));
    let port = bind_test_server(state).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{}/api/v1/UNKNOWN", port))
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}
