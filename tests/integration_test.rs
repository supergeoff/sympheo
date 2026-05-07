use std::path::PathBuf;
use symphonie::config::typed::ServiceConfig;
use symphonie::tracker::model::Issue;
use symphonie::workspace::manager::WorkspaceManager;

#[test]
fn test_workflow_loader_with_front_matter() {
    let content = r#"---
tracker:
  kind: github
  project_slug: test/repo
---
Do the work
"#;
    let wf = symphonie::workflow::parser::parse(content).unwrap();
    assert!(!wf.config.is_empty());
    assert_eq!(wf.prompt_template, "Do the work");
}

#[test]
fn test_service_config_defaults() {
    let raw = serde_yaml::Mapping::new();
    let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "prompt".into());
    assert_eq!(config.tracker_kind(), None);
    assert_eq!(config.poll_interval_ms(), 30000);
    assert_eq!(config.max_concurrent_agents(), 10);
    assert_eq!(config.max_turns(), 20);
    assert_eq!(config.codex_command(), "opencode run");
    assert_eq!(config.codex_stall_timeout_ms(), 300000);
}

#[test]
fn test_workspace_sanitization() {
    assert_eq!(
        WorkspaceManager::sanitize_identifier("ABC-123"),
        "ABC-123"
    );
    assert_eq!(
        WorkspaceManager::sanitize_identifier("feat/new_thing"),
        "feat_new_thing"
    );
    assert_eq!(
        WorkspaceManager::sanitize_identifier("bug: crash!"),
        "bug__crash_"
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
        blocked_by: vec![
            symphonie::tracker::model::BlockerRef {
                id: Some("2".into()),
                identifier: Some("TEST-2".into()),
                state: Some("in progress".into()),
            },
        ],
        created_at: None,
        updated_at: None,
    };
    let terminal = vec!["closed".into(), "done".into()];
    assert!(issue.is_blocked(&terminal));

    let unblocked = Issue {
        blocked_by: vec![
            symphonie::tracker::model::BlockerRef {
                id: Some("2".into()),
                identifier: Some("TEST-2".into()),
                state: Some("closed".into()),
            },
        ],
        ..issue
    };
    assert!(!unblocked.is_blocked(&terminal));
}

#[test]
fn test_config_var_resolution() {
    std::env::set_var("TEST_SYM_KEY", "secret123");
    assert_eq!(
        symphonie::config::resolver::resolve_value("$TEST_SYM_KEY"),
        "secret123"
    );
    assert_eq!(
        symphonie::config::resolver::resolve_value("plain"),
        "plain"
    );
}

#[test]
fn test_daytona_config_parsing() {
    use std::path::PathBuf;
    let mut raw = serde_yaml::Mapping::new();
    let mut daytona = serde_yaml::Mapping::new();
    daytona.insert(
        serde_yaml::Value::String("enabled".into()),
        serde_yaml::Value::Bool(true),
    );
    daytona.insert(
        serde_yaml::Value::String("api_key".into()),
        serde_yaml::Value::String("$DAYTONA_KEY".into()),
    );
    daytona.insert(
        serde_yaml::Value::String("endpoint".into()),
        serde_yaml::Value::String("https://api.daytona.io".into()),
    );
    raw.insert(
        serde_yaml::Value::String("daytona".into()),
        serde_yaml::Value::Mapping(daytona),
    );
    std::env::set_var("DAYTONA_KEY", "secret");
    let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "prompt".into());
    assert!(config.daytona_enabled());
    assert_eq!(config.daytona_api_key(), Some("secret".into()));
    assert_eq!(config.daytona_api_url(), "https://api.daytona.io");
}
