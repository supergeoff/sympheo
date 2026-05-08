use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tokio::time::timeout;

use crate::agent::backend::AgentBackend;
use crate::agent::parser::{AgentEvent, TokenInfo, TurnResult, parse_event_line};
use crate::config::typed::ServiceConfig;
use crate::error::SympheoError;
use crate::tracker::model::Issue;
use crate::workspace::manager::WorkspaceManager;

#[derive(Debug, Clone)]
pub struct DaytonaConfig {
    api_key: String,
    api_url: String,
    target: String,
    image: Option<String>,
    timeout_sec: u64,
    env: std::collections::HashMap<String, String>,
    command: String,
    turn_timeout_ms: u64,
    mode: String,
    repo_url: Option<String>,
}

impl DaytonaConfig {
    pub fn from_service(config: &ServiceConfig) -> Result<Self, SympheoError> {
        let _daytona_map = config.daytona().ok_or_else(|| {
            SympheoError::InvalidConfiguration(
                "daytona section required when backend is enabled".into(),
            )
        })?;

        let api_key = config
            .daytona_api_key()
            .ok_or(SympheoError::InvalidConfiguration(
                "daytona.api_key is required".into(),
            ))?;

        Ok(Self {
            api_key,
            api_url: config.daytona_api_url(),
            target: config.daytona_target(),
            image: config.daytona_image(),
            timeout_sec: config.daytona_timeout_sec(),
            env: config.daytona_env(),
            command: config.codex_command(),
            turn_timeout_ms: config.codex_turn_timeout_ms(),
            mode: config.daytona_mode(),
            repo_url: config.daytona_repo_url(),
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DaytonaSandbox {
    pub id: String,
    #[serde(default)]
    pub state: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DaytonaExecuteResponse {
    #[serde(default)]
    result: String,
    #[serde(default)]
    exit_code: i32,
}

pub struct DaytonaBackend {
    config: DaytonaConfig,
    workspace_manager: WorkspaceManager,
    client: reqwest::Client,
}

impl DaytonaBackend {
    pub fn new(service_config: &ServiceConfig) -> Result<Self, SympheoError> {
        let config = DaytonaConfig::from_service(service_config)?;
        let workspace_manager = WorkspaceManager::new(service_config)?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_sec.max(30)))
            .build()
            .map_err(|e| SympheoError::DaytonaApiError(format!("client build: {e}")))?;
        Ok(Self {
            config,
            workspace_manager,
            client,
        })
    }

    fn sandbox_meta_path(&self, workspace_path: &Path) -> std::path::PathBuf {
        workspace_path.join(".daytona_sandbox_id")
    }

    async fn read_sandbox_id(&self, workspace_path: &Path) -> Option<String> {
        let path = self.sandbox_meta_path(workspace_path);
        tokio::fs::read_to_string(&path)
            .await
            .ok()
            .map(|s| s.trim().to_string())
    }

    async fn write_sandbox_id(&self, workspace_path: &Path, id: &str) -> Result<(), SympheoError> {
        let path = self.sandbox_meta_path(workspace_path);
        tokio::fs::write(&path, id)
            .await
            .map_err(|e| SympheoError::Io(format!("write sandbox id: {e}")))?;
        Ok(())
    }

    async fn with_retry<T, F, Fut>(
        &self,
        operation: &str,
        max_retries: u32,
        f: F,
    ) -> Result<T, SympheoError>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T, SympheoError>>,
    {
        let mut last_err = None;
        for attempt in 0..=max_retries {
            if attempt > 0 {
                let delay_ms = 500u64 * 2u64.pow(attempt.saturating_sub(1));
                tokio::time::sleep(Duration::from_millis(delay_ms.min(10000))).await;
                tracing::info!(operation = %operation, attempt, "retrying daytona operation");
            }
            match f().await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    tracing::warn!(operation = %operation, attempt, error = %e, "daytona operation failed");
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            SympheoError::DaytonaApiError(format!("{operation}: max retries exceeded"))
        }))
    }

    async fn create_sandbox(&self) -> Result<DaytonaSandbox, SympheoError> {
        let url = format!("{}/api/sandbox", self.config.api_url.trim_end_matches('/'));

        let mut payload = serde_json::Map::new();
        if let Some(ref img) = self.config.image {
            payload.insert("image".to_string(), serde_json::Value::String(img.clone()));
        }
        payload.insert(
            "target".to_string(),
            serde_json::Value::String(self.config.target.clone()),
        );
        if !self.config.env.is_empty() {
            let env_map: serde_json::Map<String, serde_json::Value> = self
                .config
                .env
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                .collect();
            payload.insert("env".to_string(), serde_json::Value::Object(env_map));
        }

        tracing::info!(url = %url, "creating Daytona sandbox");

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| SympheoError::DaytonaApiError(format!("create sandbox request: {e}")))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| SympheoError::DaytonaApiError(format!("create sandbox body: {e}")))?;

        if !status.is_success() {
            return Err(SympheoError::DaytonaApiError(format!(
                "create sandbox failed: HTTP {} - {}",
                status, body
            )));
        }

        let sandbox: DaytonaSandbox = serde_json::from_str(&body).map_err(|e| {
            SympheoError::DaytonaApiError(format!("create sandbox json: {e} ({body})"))
        })?;

        tracing::info!(sandbox_id = %sandbox.id, state = %sandbox.state, "daytona sandbox created");
        Ok(sandbox)
    }

    pub async fn create_sandbox_with_retry(
        &self,
        max_retries: u32,
    ) -> Result<DaytonaSandbox, SympheoError> {
        self.with_retry("create_sandbox", max_retries, || self.create_sandbox())
            .await
    }

    pub async fn delete_sandbox(&self, sandbox_id: &str) -> Result<(), SympheoError> {
        let url = format!(
            "{}/api/sandbox/{}",
            self.config.api_url.trim_end_matches('/'),
            sandbox_id
        );
        let resp = self
            .client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .send()
            .await
            .map_err(|e| SympheoError::DaytonaApiError(format!("delete sandbox request: {e}")))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!(sandbox_id = %sandbox_id, body = %body, "failed to delete daytona sandbox");
        } else {
            tracing::info!(sandbox_id = %sandbox_id, "daytona sandbox deleted");
        }
        Ok(())
    }

    async fn get_sandbox_state(&self, sandbox_id: &str) -> Result<String, SympheoError> {
        let url = format!(
            "{}/api/sandbox/{}",
            self.config.api_url.trim_end_matches('/'),
            sandbox_id
        );

        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .send()
            .await
            .map_err(|e| {
                SympheoError::DaytonaApiError(format!("get sandbox state request: {e}"))
            })?;

        let status = resp.status();
        if status.is_success() {
            let body = resp.text().await.map_err(|e| {
                SympheoError::DaytonaApiError(format!("get sandbox state body: {e}"))
            })?;
            let sandbox: DaytonaSandbox = serde_json::from_str(&body).map_err(|e| {
                SympheoError::DaytonaApiError(format!("get sandbox state json: {e}"))
            })?;
            Ok(sandbox.state)
        } else if status.as_u16() == 404 {
            Ok("not_found".to_string())
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(SympheoError::DaytonaApiError(format!(
                "get sandbox state failed: HTTP {} - {}",
                status, body
            )))
        }
    }

    async fn start_sandbox(&self, sandbox_id: &str) -> Result<(), SympheoError> {
        let url = format!(
            "{}/api/sandbox/{}/start",
            self.config.api_url.trim_end_matches('/'),
            sandbox_id
        );

        tracing::info!(sandbox_id = %sandbox_id, "starting daytona sandbox");

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .send()
            .await
            .map_err(|e| SympheoError::DaytonaApiError(format!("start sandbox request: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(SympheoError::DaytonaApiError(format!(
                "start sandbox failed: HTTP {} - {}",
                status, body
            )));
        }

        tracing::info!(sandbox_id = %sandbox_id, "daytona sandbox started");
        Ok(())
    }

    pub async fn ensure_sandbox_running(
        &self,
        workspace_path: &Path,
    ) -> Result<String, SympheoError> {
        if let Some(id) = self.read_sandbox_id(workspace_path).await {
            let state = self.get_sandbox_state(&id).await?;
            match state.as_str() {
                "running" => {
                    tracing::info!(sandbox_id = %id, "reusing existing daytona sandbox");
                    return Ok(id);
                }
                "stopped" | "created" => {
                    tracing::info!(sandbox_id = %id, state = %state, "starting existing daytona sandbox");
                    self.start_sandbox(&id).await?;
                    return Ok(id);
                }
                "error" | "not_found" => {
                    tracing::warn!(sandbox_id = %id, state = %state, "sandbox in bad state, recreating");
                    let _ = tokio::fs::remove_file(self.sandbox_meta_path(workspace_path)).await;
                }
                _ => {
                    tracing::warn!(sandbox_id = %id, state = %state, "unknown sandbox state, recreating");
                    let _ = tokio::fs::remove_file(self.sandbox_meta_path(workspace_path)).await;
                }
            }
        }

        let sandbox = self.create_sandbox_with_retry(3).await?;
        self.write_sandbox_id(workspace_path, &sandbox.id).await?;

        if sandbox.state != "running" {
            self.start_sandbox(&sandbox.id).await?;
        }

        tracing::info!(sandbox_id = %sandbox.id, "daytona sandbox created and running");
        Ok(sandbox.id)
    }

    async fn execute_command(
        &self,
        sandbox_id: &str,
        command: &str,
        cwd: &str,
    ) -> Result<DaytonaExecuteResponse, SympheoError> {
        let url = format!(
            "https://proxy.app.daytona.io/toolbox/{}/process/execute",
            sandbox_id
        );

        let payload = serde_json::json!({
            "command": command,
            "cwd": cwd,
            "timeout": self.config.timeout_sec as u32,
        });

        tracing::debug!(sandbox_id = %sandbox_id, command = %command, "executing in daytona sandbox");

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| SympheoError::DaytonaApiError(format!("execute request: {e}")))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| SympheoError::DaytonaApiError(format!("execute body: {e}")))?;

        if !status.is_success() {
            return Err(SympheoError::DaytonaApiError(format!(
                "execute failed: HTTP {} - {}",
                status, body
            )));
        }

        let result: DaytonaExecuteResponse = serde_json::from_str(&body)
            .map_err(|e| SympheoError::DaytonaApiError(format!("execute json: {e} ({body})")))?;

        Ok(result)
    }

    async fn sync_workspace_to_sandbox(&self, sandbox_id: &str) -> Result<(), SympheoError> {
        let check = self
            .execute_command(sandbox_id, "ls -A /workspace", "/")
            .await?;
        if check.exit_code == 0 && !check.result.trim().is_empty() {
            tracing::info!(sandbox_id = %sandbox_id, "workspace already populated in sandbox");
            return Ok(());
        }

        if let Some(ref repo_url) = self.config.repo_url {
            tracing::info!(sandbox_id = %sandbox_id, repo_url = %repo_url, "cloning repo into sandbox workspace");
            let clone_result = self
                .execute_command(
                    sandbox_id,
                    &format!("git clone {} /workspace", shell_escape(repo_url)),
                    "/",
                )
                .await?;
            if clone_result.exit_code != 0 {
                tracing::warn!(sandbox_id = %sandbox_id, error = %clone_result.result, "git clone failed in sandbox");
            }
        } else {
            tracing::debug!(sandbox_id = %sandbox_id, "no repo_url configured, leaving sandbox workspace empty");
        }
        Ok(())
    }
}

#[async_trait]
impl AgentBackend for DaytonaBackend {
    async fn run_turn(
        &self,
        issue: &Issue,
        prompt: &str,
        session_id: Option<&str>,
        workspace_path: &Path,
        _cancelled: Arc<AtomicBool>,
    ) -> Result<(TurnResult, tokio::sync::mpsc::Receiver<AgentEvent>), SympheoError> {
        self.workspace_manager
            .validate_inside_root(workspace_path)?;

        let sandbox_id = self.ensure_sandbox_running(workspace_path).await?;

        if let Err(e) = self.sync_workspace_to_sandbox(&sandbox_id).await {
            tracing::warn!(error = %e, "workspace sync failed, continuing anyway");
        }

        let sandbox_dir = "/workspace";
        let mut opencode_cmd = format!(
            r#"{} "{}" --format json --dir {} --dangerously-skip-permissions"#,
            self.config.command,
            shell_escape(prompt),
            shell_escape(sandbox_dir)
        );
        if let Some(sid) = session_id {
            opencode_cmd.push_str(&format!(" --session {}", shell_escape(sid)));
        }

        tracing::info!(
            issue_id = %issue.id,
            issue_identifier = %issue.identifier,
            sandbox_id = %sandbox_id,
            mode = %self.config.mode,
            has_session = session_id.is_some(),
            "launching opencode agent (daytona backend)"
        );

        let exec_result = timeout(
            Duration::from_millis(self.config.turn_timeout_ms),
            self.execute_command(&sandbox_id, &opencode_cmd, sandbox_dir),
        )
        .await;

        let exec = match exec_result {
            Ok(Ok(exec)) => exec,
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(SympheoError::AgentTurnTimeout),
        };

        if exec.exit_code != 0 {
            return Err(SympheoError::AgentRunnerError(format!(
                "daytona process exited with code {}: {}",
                exec.exit_code, exec.result
            )));
        }

        let (event_tx, event_rx) = tokio::sync::mpsc::channel(100);

        let mut current_session: Option<String> = None;
        let mut current_turn: Option<String> = None;
        let mut accumulated_text = String::new();
        let mut tokens: Option<TokenInfo> = None;
        let mut success = false;

        for line in exec.result.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Some(event) = parse_event_line(line) {
                match &event {
                    AgentEvent::StepStart {
                        session_id, part, ..
                    } => {
                        current_session = Some(session_id.clone());
                        current_turn = Some(part.message_id.clone());
                    }
                    AgentEvent::Text { part, .. } => {
                        accumulated_text.push_str(&part.text);
                    }
                    AgentEvent::StepFinish { part, .. } => {
                        tokens = part.tokens.clone();
                        success = part.reason == "stop" || part.reason == "tool-calls";
                    }
                    _ => {}
                }
                let _ = event_tx.send(event).await;
            }
        }

        let sid = current_session.unwrap_or_else(|| issue.id.clone());
        let tid = current_turn.unwrap_or_else(|| "turn-1".into());

        Ok((
            TurnResult {
                session_id: sid.clone(),
                turn_id: tid,
                success,
                text: accumulated_text,
                tokens,
            },
            event_rx,
        ))
    }

    async fn cleanup_workspace(&self, workspace_path: &Path) -> Result<(), SympheoError> {
        if let Some(id) = self.read_sandbox_id(workspace_path).await {
            tracing::info!(sandbox_id = %id, "cleaning up daytona sandbox");
            if let Err(e) = self.delete_sandbox(&id).await {
                tracing::warn!(sandbox_id = %id, error = %e, "failed to delete sandbox during cleanup");
            }
            let meta_path = self.sandbox_meta_path(workspace_path);
            if let Err(e) = tokio::fs::remove_file(&meta_path).await {
                tracing::debug!(path = %meta_path.display(), error = %e, "failed to remove sandbox meta file");
            }
        }
        Ok(())
    }
}

fn shell_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn service_config_with_daytona(api_url: &str) -> ServiceConfig {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut daytona = serde_json::Map::<String, serde_json::Value>::new();
        daytona.insert("enabled".into(), serde_json::Value::Bool(true));
        daytona.insert(
            "api_key".into(),
            serde_json::Value::String("test-key".into()),
        );
        daytona.insert("api_url".into(), serde_json::Value::String(api_url.into()));
        daytona.insert("target".into(), serde_json::Value::String("eu".into()));
        daytona.insert(
            "image".into(),
            serde_json::Value::String("custom-image".into()),
        );
        daytona.insert("timeout_sec".into(), serde_json::Value::Number(7200.into()));
        let mut env = serde_json::Map::<String, serde_json::Value>::new();
        env.insert("FOO".into(), serde_json::Value::String("bar".into()));
        daytona.insert("env".into(), serde_json::Value::Object(env));
        raw.insert("daytona".into(), serde_json::Value::Object(daytona));
        let mut codex = serde_json::Map::<String, serde_json::Value>::new();
        codex.insert(
            "command".into(),
            serde_json::Value::String("opencode run".into()),
        );
        raw.insert("codex".into(), serde_json::Value::Object(codex));
        ServiceConfig::new(raw, PathBuf::from("/tmp"), "prompt".into())
    }

    #[test]
    fn test_daytona_config_from_service() {
        let config = service_config_with_daytona("https://custom.daytona.io");
        let dc = DaytonaConfig::from_service(&config).unwrap();
        assert_eq!(dc.api_key, "test-key");
        assert_eq!(dc.api_url, "https://custom.daytona.io");
        assert_eq!(dc.target, "eu");
        assert_eq!(dc.image, Some("custom-image".into()));
        assert_eq!(dc.timeout_sec, 7200);
        assert_eq!(dc.command, "opencode run");
        assert_eq!(dc.env.get("FOO"), Some(&"bar".to_string()));
        assert_eq!(dc.mode, "oneshot");
        assert_eq!(dc.repo_url, None);
    }

    #[test]
    fn test_daytona_config_mode_and_repo_url() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut daytona = serde_json::Map::<String, serde_json::Value>::new();
        daytona.insert("enabled".into(), serde_json::Value::Bool(true));
        daytona.insert(
            "api_key".into(),
            serde_json::Value::String("test-key".into()),
        );
        daytona.insert("mode".into(), serde_json::Value::String("AppServer".into()));
        daytona.insert(
            "repo_url".into(),
            serde_json::Value::String("https://github.com/test/repo.git".into()),
        );
        raw.insert("daytona".into(), serde_json::Value::Object(daytona));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "prompt".into());
        let dc = DaytonaConfig::from_service(&config).unwrap();
        assert_eq!(dc.mode, "appserver");
        assert_eq!(
            dc.repo_url,
            Some("https://github.com/test/repo.git".to_string())
        );
    }

    #[test]
    fn test_daytona_config_missing_section() {
        let config = ServiceConfig::new(
            serde_json::Map::<String, serde_json::Value>::new(),
            PathBuf::from("/tmp"),
            "".into(),
        );
        let result = DaytonaConfig::from_service(&config);
        assert!(matches!(result, Err(SympheoError::InvalidConfiguration(_))));
    }

    #[test]
    fn test_daytona_config_missing_api_key() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut daytona = serde_json::Map::<String, serde_json::Value>::new();
        daytona.insert("enabled".into(), serde_json::Value::Bool(true));
        raw.insert("daytona".into(), serde_json::Value::Object(daytona));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let result = DaytonaConfig::from_service(&config);
        assert!(matches!(result, Err(SympheoError::InvalidConfiguration(_))));
    }

    #[test]
    fn test_daytona_backend_new() {
        let config = service_config_with_daytona("https://custom.daytona.io");
        let backend = DaytonaBackend::new(&config).unwrap();
        assert_eq!(backend.config.api_key, "test-key");
    }

    #[test]
    fn test_shell_escape_daytona() {
        assert_eq!(shell_escape("a\\b"), "a\\\\b");
        assert_eq!(shell_escape("\""), "\\\"");
        assert_eq!(shell_escape("$"), "\\$");
        assert_eq!(shell_escape("`"), "\\`");
    }

    #[test]
    fn test_sandbox_meta_path() {
        let config = service_config_with_daytona("https://custom.daytona.io");
        let backend = DaytonaBackend::new(&config).unwrap();
        let path = backend.sandbox_meta_path(Path::new("/workspace"));
        assert_eq!(path, PathBuf::from("/workspace/.daytona_sandbox_id"));
    }

    #[tokio::test]
    async fn test_create_sandbox_success() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/api/sandbox"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "sandbox-123",
                    "state": "running"
                })),
            )
            .mount(&mock_server)
            .await;

        let config = service_config_with_daytona(&mock_server.uri());
        let backend = DaytonaBackend::new(&config).unwrap();
        let sandbox = backend.create_sandbox().await.unwrap();
        assert_eq!(sandbox.id, "sandbox-123");
        assert_eq!(sandbox.state, "running");
    }

    #[tokio::test]
    async fn test_create_sandbox_failure() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/api/sandbox"))
            .respond_with(wiremock::ResponseTemplate::new(500).set_body_string("error"))
            .mount(&mock_server)
            .await;

        let config = service_config_with_daytona(&mock_server.uri());
        let backend = DaytonaBackend::new(&config).unwrap();
        let result = backend.create_sandbox().await;
        assert!(matches!(result, Err(SympheoError::DaytonaApiError(_))));
    }

    #[tokio::test]
    async fn test_delete_sandbox_success() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("DELETE"))
            .and(wiremock::matchers::path("/api/sandbox/abc"))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let config = service_config_with_daytona(&mock_server.uri());
        let backend = DaytonaBackend::new(&config).unwrap();
        let result = backend.delete_sandbox("abc").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_get_sandbox_state_running() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/sandbox/sb-1"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "sb-1",
                    "state": "running"
                })),
            )
            .mount(&mock_server)
            .await;

        let config = service_config_with_daytona(&mock_server.uri());
        let backend = DaytonaBackend::new(&config).unwrap();
        let state = backend.get_sandbox_state("sb-1").await.unwrap();
        assert_eq!(state, "running");
    }

    #[tokio::test]
    async fn test_get_sandbox_state_not_found() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/sandbox/sb-missing"))
            .respond_with(wiremock::ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let config = service_config_with_daytona(&mock_server.uri());
        let backend = DaytonaBackend::new(&config).unwrap();
        let state = backend.get_sandbox_state("sb-missing").await.unwrap();
        assert_eq!(state, "not_found");
    }

    #[tokio::test]
    async fn test_start_sandbox_success() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/api/sandbox/sb-2/start"))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let config = service_config_with_daytona(&mock_server.uri());
        let backend = DaytonaBackend::new(&config).unwrap();
        let result = backend.start_sandbox("sb-2").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_ensure_sandbox_running_create_new() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/api/sandbox"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "sandbox-new",
                    "state": "running"
                })),
            )
            .mount(&mock_server)
            .await;

        let config = service_config_with_daytona(&mock_server.uri());
        let backend = DaytonaBackend::new(&config).unwrap();
        let tmp = std::env::temp_dir().join(format!("sympheo_test_ensure_{}", std::process::id()));
        let _ = tokio::fs::create_dir_all(&tmp).await;
        let id = backend.ensure_sandbox_running(&tmp).await.unwrap();
        assert_eq!(id, "sandbox-new");
        let meta = tokio::fs::read_to_string(tmp.join(".daytona_sandbox_id"))
            .await
            .unwrap();
        assert_eq!(meta.trim(), "sandbox-new");
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    #[tokio::test]
    async fn test_ensure_sandbox_running_reuse_existing() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/sandbox/sandbox-existing"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "sandbox-existing",
                    "state": "running"
                })),
            )
            .mount(&mock_server)
            .await;

        let config = service_config_with_daytona(&mock_server.uri());
        let backend = DaytonaBackend::new(&config).unwrap();
        let tmp = std::env::temp_dir().join(format!("sympheo_test_reuse_{}", std::process::id()));
        let _ = tokio::fs::create_dir_all(&tmp).await;
        tokio::fs::write(tmp.join(".daytona_sandbox_id"), "sandbox-existing")
            .await
            .unwrap();
        let id = backend.ensure_sandbox_running(&tmp).await.unwrap();
        assert_eq!(id, "sandbox-existing");
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    #[tokio::test]
    async fn test_ensure_sandbox_running_start_stopped() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/sandbox/sandbox-stopped"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "sandbox-stopped",
                    "state": "stopped"
                })),
            )
            .mount(&mock_server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path(
                "/api/sandbox/sandbox-stopped/start",
            ))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let config = service_config_with_daytona(&mock_server.uri());
        let backend = DaytonaBackend::new(&config).unwrap();
        let tmp = std::env::temp_dir().join(format!("sympheo_test_start_{}", std::process::id()));
        let _ = tokio::fs::create_dir_all(&tmp).await;
        tokio::fs::write(tmp.join(".daytona_sandbox_id"), "sandbox-stopped")
            .await
            .unwrap();
        let id = backend.ensure_sandbox_running(&tmp).await.unwrap();
        assert_eq!(id, "sandbox-stopped");
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    #[tokio::test]
    async fn test_ensure_sandbox_running_recreate_error() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/sandbox/sandbox-error"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "sandbox-error",
                    "state": "error"
                })),
            )
            .mount(&mock_server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/api/sandbox"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "sandbox-recreated",
                    "state": "running"
                })),
            )
            .mount(&mock_server)
            .await;

        let config = service_config_with_daytona(&mock_server.uri());
        let backend = DaytonaBackend::new(&config).unwrap();
        let tmp =
            std::env::temp_dir().join(format!("sympheo_test_recreate_{}", std::process::id()));
        let _ = tokio::fs::create_dir_all(&tmp).await;
        tokio::fs::write(tmp.join(".daytona_sandbox_id"), "sandbox-error")
            .await
            .unwrap();
        let id = backend.ensure_sandbox_running(&tmp).await.unwrap();
        assert_eq!(id, "sandbox-recreated");
        let meta = tokio::fs::read_to_string(tmp.join(".daytona_sandbox_id"))
            .await
            .unwrap();
        assert_eq!(meta.trim(), "sandbox-recreated");
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    #[tokio::test]
    async fn test_cleanup_workspace() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("DELETE"))
            .and(wiremock::matchers::path("/api/sandbox/sandbox-to-clean"))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let config = service_config_with_daytona(&mock_server.uri());
        let backend = DaytonaBackend::new(&config).unwrap();
        let tmp = std::env::temp_dir().join(format!("sympheo_test_cleanup_{}", std::process::id()));
        let _ = tokio::fs::create_dir_all(&tmp).await;
        tokio::fs::write(tmp.join(".daytona_sandbox_id"), "sandbox-to-clean")
            .await
            .unwrap();
        backend.cleanup_workspace(&tmp).await.unwrap();
        assert!(!tmp.join(".daytona_sandbox_id").exists());
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    #[tokio::test]
    async fn test_retry_eventually_succeeds() {
        let mock_server = wiremock::MockServer::start().await;
        let req_count = std::sync::atomic::AtomicUsize::new(0);
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/api/sandbox"))
            .respond_with(move |_: &wiremock::Request| {
                let count = req_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if count < 2 {
                    wiremock::ResponseTemplate::new(500).set_body_string("server error")
                } else {
                    wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "id": "sandbox-retry",
                        "state": "running"
                    }))
                }
            })
            .mount(&mock_server)
            .await;

        let config = service_config_with_daytona(&mock_server.uri());
        let backend = DaytonaBackend::new(&config).unwrap();
        let sandbox = backend.create_sandbox_with_retry(3).await.unwrap();
        assert_eq!(sandbox.id, "sandbox-retry");
    }

    #[tokio::test]
    async fn test_retry_exhausted_fails() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/api/sandbox"))
            .respond_with(wiremock::ResponseTemplate::new(500).set_body_string("error"))
            .mount(&mock_server)
            .await;

        let config = service_config_with_daytona(&mock_server.uri());
        let backend = DaytonaBackend::new(&config).unwrap();
        let result = backend.create_sandbox_with_retry(2).await;
        assert!(matches!(result, Err(SympheoError::DaytonaApiError(_))));
    }
}
