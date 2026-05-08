use std::path::PathBuf;
use sympheo::agent::backend::daytona::DaytonaBackend;
use sympheo::agent::backend::AgentBackend;
use sympheo::agent::runner::AgentRunner;
use sympheo::config::typed::ServiceConfig;
fn daytona_service_config(api_url: &str) -> ServiceConfig {
    let mut raw = serde_yaml::Mapping::new();
    let mut daytona = serde_yaml::Mapping::new();
    daytona.insert(
        serde_yaml::Value::String("enabled".into()),
        serde_yaml::Value::Bool(true),
    );
    daytona.insert(
        serde_yaml::Value::String("api_key".into()),
        serde_yaml::Value::String("test-key".into()),
    );
    daytona.insert(
        serde_yaml::Value::String("api_url".into()),
        serde_yaml::Value::String(api_url.into()),
    );
    daytona.insert(
        serde_yaml::Value::String("target".into()),
        serde_yaml::Value::String("eu".into()),
    );
    raw.insert(
        serde_yaml::Value::String("daytona".into()),
        serde_yaml::Value::Mapping(daytona),
    );
    let mut workspace = serde_yaml::Mapping::new();
    workspace.insert(
        serde_yaml::Value::String("root".into()),
        serde_yaml::Value::String(std::env::temp_dir().to_string_lossy().to_string()),
    );
    raw.insert(
        serde_yaml::Value::String("workspace".into()),
        serde_yaml::Value::Mapping(workspace),
    );
    ServiceConfig::new(raw, PathBuf::from("/tmp"), "prompt".into())
}

#[test]
fn test_agent_runner_selects_daytona() {
    let config = daytona_service_config("https://api.daytona.io");
    let runner = AgentRunner::new(&config);
    assert!(runner.is_ok());
}

#[tokio::test]
async fn test_daytona_backend_cleanup_reads_meta_file() {
    let mock_server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("DELETE"))
        .and(wiremock::matchers::path("/api/sandbox/sb-cleanup"))
        .respond_with(wiremock::ResponseTemplate::new(204))
        .mount(&mock_server)
        .await;

    let config = daytona_service_config(&mock_server.uri());
    let backend = DaytonaBackend::new(&config).unwrap();

    let tmp = std::env::temp_dir().join(format!(
        "sympheo_int_cleanup_{}",
        std::process::id()
    ));
    tokio::fs::create_dir_all(&tmp).await.unwrap();
    tokio::fs::write(tmp.join(".daytona_sandbox_id"), "sb-cleanup")
        .await
        .unwrap();

    backend.cleanup_workspace(&tmp).await.unwrap();

    assert!(!tmp.join(".daytona_sandbox_id").exists());
    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

#[tokio::test]
async fn test_daytona_backend_lifecycle_create_start_delete() {
    let mock_server = wiremock::MockServer::start().await;

    // Create sandbox
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/sandbox"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "sb-lifecycle",
            "state": "created"
        })))
        .mount(&mock_server)
        .await;

    // Start sandbox
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/sandbox/sb-lifecycle/start"))
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    // Get state (after start)
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/api/sandbox/sb-lifecycle"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "sb-lifecycle",
            "state": "running"
        })))
        .mount(&mock_server)
        .await;

    // Delete sandbox
    wiremock::Mock::given(wiremock::matchers::method("DELETE"))
        .and(wiremock::matchers::path("/api/sandbox/sb-lifecycle"))
        .respond_with(wiremock::ResponseTemplate::new(204))
        .mount(&mock_server)
        .await;

    let config = daytona_service_config(&mock_server.uri());
    let backend = DaytonaBackend::new(&config).unwrap();

    let tmp = std::env::temp_dir().join(format!(
        "sympheo_int_lifecycle_{}",
        std::process::id()
    ));
    tokio::fs::create_dir_all(&tmp).await.unwrap();

    // ensure_sandbox_running should create + start
    let id = sympheo::agent::backend::daytona::DaytonaBackend::ensure_sandbox_running(
        &backend,
        &tmp,
    )
    .await
    .unwrap();
    assert_eq!(id, "sb-lifecycle");

    // cleanup should delete
    backend.cleanup_workspace(&tmp).await.unwrap();

    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

#[tokio::test]
async fn test_daytona_backend_session_reuse() {
    let mock_server = wiremock::MockServer::start().await;

    // First get state call
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/api/sandbox/sb-reuse"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "sb-reuse",
            "state": "running"
        })))
        .mount(&mock_server)
        .await;

    let config = daytona_service_config(&mock_server.uri());
    let backend = DaytonaBackend::new(&config).unwrap();

    let tmp = std::env::temp_dir().join(format!(
        "sympheo_int_reuse_{}",
        std::process::id()
    ));
    tokio::fs::create_dir_all(&tmp).await.unwrap();
    tokio::fs::write(tmp.join(".daytona_sandbox_id"), "sb-reuse")
        .await
        .unwrap();

    let id = sympheo::agent::backend::daytona::DaytonaBackend::ensure_sandbox_running(
        &backend,
        &tmp,
    )
    .await
    .unwrap();
    assert_eq!(id, "sb-reuse");

    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

#[tokio::test]
async fn test_daytona_backend_unstartable_sandbox_recreate() {
    let mock_server = wiremock::MockServer::start().await;

    // First: existing sandbox is in error state
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/api/sandbox/sb-error"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "sb-error",
            "state": "error"
        })))
        .mount(&mock_server)
        .await;

    // Then: create new sandbox
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/sandbox"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "sb-new",
            "state": "running"
        })))
        .mount(&mock_server)
        .await;

    let config = daytona_service_config(&mock_server.uri());
    let backend = DaytonaBackend::new(&config).unwrap();

    let tmp = std::env::temp_dir().join(format!(
        "sympheo_int_error_{}",
        std::process::id()
    ));
    tokio::fs::create_dir_all(&tmp).await.unwrap();
    tokio::fs::write(tmp.join(".daytona_sandbox_id"), "sb-error")
        .await
        .unwrap();

    let id = sympheo::agent::backend::daytona::DaytonaBackend::ensure_sandbox_running(
        &backend,
        &tmp,
    )
    .await
    .unwrap();
    assert_eq!(id, "sb-new");

    let meta = tokio::fs::read_to_string(tmp.join(".daytona_sandbox_id"))
        .await
        .unwrap();
    assert_eq!(meta.trim(), "sb-new");

    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

#[tokio::test]
async fn test_daytona_backend_api_down_retry_then_fail() {
    let mock_server = wiremock::MockServer::start().await;

    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/sandbox"))
        .respond_with(wiremock::ResponseTemplate::new(503).set_body_string("down"))
        .mount(&mock_server)
        .await;

    let config = daytona_service_config(&mock_server.uri());
    let backend = DaytonaBackend::new(&config).unwrap();

    let tmp = std::env::temp_dir().join(format!(
        "sympheo_int_down_{}",
        std::process::id()
    ));
    tokio::fs::create_dir_all(&tmp).await.unwrap();

    let result = sympheo::agent::backend::daytona::DaytonaBackend::create_sandbox_with_retry(
        &backend, 2,
    )
    .await;
    assert!(result.is_err());

    let _ = tokio::fs::remove_dir_all(&tmp).await;
}
