use crate::agent::backend::{daytona::DaytonaBackend, local::LocalBackend};
use crate::agent::backend::AgentBackend;
use crate::agent::parser::{AgentEvent, TurnResult};
use tokio::sync::mpsc::Receiver;
use crate::config::typed::ServiceConfig;
use crate::error::SympheoError;
use crate::tracker::model::Issue;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub struct AgentRunner {
    backend: Arc<dyn AgentBackend>,
}

impl AgentRunner {
    pub fn new(config: &ServiceConfig) -> Result<Self, SympheoError> {
        let backend: Arc<dyn AgentBackend> = if config.daytona_enabled() {
            Arc::new(DaytonaBackend::new(config)?)
        } else {
            Arc::new(LocalBackend::new(config)?)
        };
        Ok(Self { backend })
    }

    pub async fn run_turn(
        &self,
        issue: &Issue,
        prompt: &str,
        session_id: Option<&str>,
        workspace_path: &Path,
        cancelled: Arc<AtomicBool>,
    ) -> Result<(TurnResult, Receiver<AgentEvent>), SympheoError> {
        self.backend.run_turn(issue, prompt, session_id, workspace_path, cancelled).await
    }

    pub async fn cleanup_workspace(&self, workspace_path: &Path) -> Result<(), SympheoError> {
        self.backend.cleanup_workspace(workspace_path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn base_config() -> serde_json::Map<String, serde_json::Value> {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut workspace = serde_json::Map::<String, serde_json::Value>::new();
        workspace.insert(
            "root".into(),
            serde_json::Value::String("/tmp".into()),
        );
        raw.insert(
            "workspace".into(),
            serde_json::Value::Object(workspace),
        );
        raw
    }

    #[test]
    fn test_agent_runner_local_success() {
        let mut raw = base_config();
        let mut codex = serde_json::Map::<String, serde_json::Value>::new();
        codex.insert(
            "command".into(),
            serde_json::Value::String("echo".into()),
        );
        raw.insert(
            "codex".into(),
            serde_json::Value::Object(codex),
        );
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let runner = AgentRunner::new(&config);
        assert!(runner.is_ok());
    }

    #[test]
    fn test_agent_runner_daytona_success() {
        let mut raw = base_config();
        let mut daytona = serde_json::Map::<String, serde_json::Value>::new();
        daytona.insert(
            "enabled".into(),
            serde_json::Value::Bool(true),
        );
        daytona.insert(
            "api_key".into(),
            serde_json::Value::String("test-key".into()),
        );
        daytona.insert(
            "server_url".into(),
            serde_json::Value::String("http://localhost".into()),
        );
        daytona.insert(
            "target".into(),
            serde_json::Value::String("local".into()),
        );
        raw.insert(
            "daytona".into(),
            serde_json::Value::Object(daytona),
        );
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let runner = AgentRunner::new(&config);
        assert!(runner.is_ok());
    }

    #[test]
    fn test_agent_runner_daytona_failure() {
        let mut raw = base_config();
        let mut daytona = serde_json::Map::<String, serde_json::Value>::new();
        daytona.insert(
            "enabled".into(),
            serde_json::Value::Bool(true),
        );
        // Missing api_key
        raw.insert(
            "daytona".into(),
            serde_json::Value::Object(daytona),
        );
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let runner = AgentRunner::new(&config);
        assert!(runner.is_err());
    }
}
