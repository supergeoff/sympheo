use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use tokio::time::timeout;

use crate::agent::backend::AgentBackend;
use crate::agent::parser::{parse_line, OpencodeEvent, TokenInfo, TurnResult};
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
}

impl DaytonaConfig {
    pub fn from_service(config: &ServiceConfig) -> Result<Self, SympheoError> {
        let _daytona_map = config
            .daytona()
            .ok_or_else(|| SympheoError::InvalidConfiguration(
                "daytona section required when backend is enabled".into(),
            ))?;

        let api_key = config.daytona_api_key()
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
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct DaytonaSandbox {
    id: String,
    #[serde(default)]
    state: String,
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
        tokio::fs::read_to_string(&path).await.ok().map(|s| s.trim().to_string())
    }

    async fn write_sandbox_id(&self, workspace_path: &Path, id: &str) -> Result<(), SympheoError> {
        let path = self.sandbox_meta_path(workspace_path);
        tokio::fs::write(&path, id)
            .await
            .map_err(|e| SympheoError::Io(format!("write sandbox id: {e}")))?;
        Ok(())
    }

    async fn create_sandbox(&self) -> Result<DaytonaSandbox, SympheoError> {
        let url = format!("{}/api/sandbox", self.config.api_url.trim_end_matches('/'));

        let mut payload = serde_json::Map::new();
        if let Some(ref img) = self.config.image {
            payload.insert("image".to_string(), serde_json::Value::String(img.clone()));
        }
        payload.insert("target".to_string(), serde_json::Value::String(self.config.target.clone()));
        if !self.config.env.is_empty() {
            let env_map: serde_json::Map<String, serde_json::Value> = self.config
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

        let sandbox: DaytonaSandbox = serde_json::from_str(&body)
            .map_err(|e| SympheoError::DaytonaApiError(format!("create sandbox json: {e} ({body})")))?;

        tracing::info!(sandbox_id = %sandbox.id, "daytona sandbox created");
        Ok(sandbox)
    }

    #[allow(dead_code)]
    async fn delete_sandbox(&self, sandbox_id: &str) -> Result<(), SympheoError> {
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
}

#[async_trait]
impl AgentBackend for DaytonaBackend {
    async fn run_turn(
        &self,
        issue: &Issue,
        prompt: &str,
        session_id: Option<&str>,
        workspace_path: &Path,
    ) -> Result<TurnResult, SympheoError> {
        self.workspace_manager
            .validate_inside_root(workspace_path)?;

        let sandbox_id = match self.read_sandbox_id(workspace_path).await {
            Some(id) => id,
            None => {
                let sandbox = self.create_sandbox().await?;
                self.write_sandbox_id(workspace_path, &sandbox.id).await?;
                sandbox.id
            }
        };

        let mut opencode_cmd = format!(
            r#"{} "{}" --format json --dir {} --dangerously-skip-permissions"#,
            self.config.command,
            shell_escape(prompt),
            shell_escape(&workspace_path.to_string_lossy())
        );
        if let Some(sid) = session_id {
            opencode_cmd.push_str(&format!(" --session {}", shell_escape(sid)));
        }

        tracing::info!(
            issue_id = %issue.id,
            issue_identifier = %issue.identifier,
            sandbox_id = %sandbox_id,
            "launching opencode agent (daytona backend)"
        );

        let exec_result = timeout(
            Duration::from_millis(self.config.turn_timeout_ms),
            self.execute_command(&sandbox_id, &opencode_cmd, "/workspace"),
        )
        .await
        .map_err(|_| SympheoError::AgentTurnTimeout)?;

        let exec = exec_result?;

        if exec.exit_code != 0 {
            return Err(SympheoError::AgentRunnerError(format!(
                "daytona process exited with code {}: {}",
                exec.exit_code, exec.result
            )));
        }

        let mut current_session: Option<String> = None;
        let mut current_turn: Option<String> = None;
        let mut accumulated_text = String::new();
        let mut tokens: Option<TokenInfo> = None;
        let mut success = false;

        for line in exec.result.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match parse_line(line) {
                Some(OpencodeEvent::StepStart { session_id, part, .. }) => {
                    current_session = Some(session_id);
                    current_turn = Some(part.message_id.clone());
                }
                Some(OpencodeEvent::Text { part, .. }) => {
                    accumulated_text.push_str(&part.text);
                }
                Some(OpencodeEvent::StepFinish { part, .. }) => {
                    tokens = part.tokens.clone();
                    success = part.reason == "stop" || part.reason == "tool-calls";
                    break;
                }
                _ => {}
            }
        }

        let sid = current_session.unwrap_or_else(|| issue.id.clone());
        let tid = current_turn.unwrap_or_else(|| "turn-1".into());

        Ok(TurnResult {
            session_id: sid.clone(),
            turn_id: tid,
            success,
            text: accumulated_text,
            tokens,
        })
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
        daytona.insert(
            serde_yaml::Value::String("image".into()),
            serde_yaml::Value::String("custom-image".into()),
        );
        daytona.insert(
            serde_yaml::Value::String("timeout_sec".into()),
            serde_yaml::Value::Number(7200.into()),
        );
        let mut env = serde_yaml::Mapping::new();
        env.insert(
            serde_yaml::Value::String("FOO".into()),
            serde_yaml::Value::String("bar".into()),
        );
        daytona.insert(
            serde_yaml::Value::String("env".into()),
            serde_yaml::Value::Mapping(env),
        );
        raw.insert(
            serde_yaml::Value::String("daytona".into()),
            serde_yaml::Value::Mapping(daytona),
        );
        let mut codex = serde_yaml::Mapping::new();
        codex.insert(
            serde_yaml::Value::String("command".into()),
            serde_yaml::Value::String("opencode run".into()),
        );
        raw.insert(
            serde_yaml::Value::String("codex".into()),
            serde_yaml::Value::Mapping(codex),
        );
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
    }

    #[test]
    fn test_daytona_config_missing_section() {
        let config = ServiceConfig::new(serde_yaml::Mapping::new(), PathBuf::from("/tmp"), "".into());
        let result = DaytonaConfig::from_service(&config);
        assert!(matches!(result, Err(SympheoError::InvalidConfiguration(_))));
    }

    #[test]
    fn test_daytona_config_missing_api_key() {
        let mut raw = serde_yaml::Mapping::new();
        let mut daytona = serde_yaml::Mapping::new();
        daytona.insert(
            serde_yaml::Value::String("enabled".into()),
            serde_yaml::Value::Bool(true),
        );
        raw.insert(
            serde_yaml::Value::String("daytona".into()),
            serde_yaml::Value::Mapping(daytona),
        );
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
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "sandbox-123",
                "state": "running"
            })))
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
}
