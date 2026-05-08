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

    fn base_config() -> serde_yaml::Mapping {
        let mut raw = serde_yaml::Mapping::new();
        let mut workspace = serde_yaml::Mapping::new();
        workspace.insert(
            serde_yaml::Value::String("root".into()),
            serde_yaml::Value::String("/tmp".into()),
        );
        raw.insert(
            serde_yaml::Value::String("workspace".into()),
            serde_yaml::Value::Mapping(workspace),
        );
        raw
    }

    #[test]
    fn test_agent_runner_local_success() {
        let mut raw = base_config();
        let mut codex = serde_yaml::Mapping::new();
        codex.insert(
            serde_yaml::Value::String("command".into()),
            serde_yaml::Value::String("echo".into()),
        );
        raw.insert(
            serde_yaml::Value::String("codex".into()),
            serde_yaml::Value::Mapping(codex),
        );
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let runner = AgentRunner::new(&config);
        assert!(runner.is_ok());
    }

    #[test]
    fn test_agent_runner_daytona_success() {
        let mut raw = base_config();
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
            serde_yaml::Value::String("server_url".into()),
            serde_yaml::Value::String("http://localhost".into()),
        );
        daytona.insert(
            serde_yaml::Value::String("target".into()),
            serde_yaml::Value::String("local".into()),
        );
        raw.insert(
            serde_yaml::Value::String("daytona".into()),
            serde_yaml::Value::Mapping(daytona),
        );
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let runner = AgentRunner::new(&config);
        assert!(runner.is_ok());
    }

    #[test]
    fn test_agent_runner_daytona_failure() {
        let mut raw = base_config();
        let mut daytona = serde_yaml::Mapping::new();
        daytona.insert(
            serde_yaml::Value::String("enabled".into()),
            serde_yaml::Value::Bool(true),
        );
        // Missing api_key
        raw.insert(
            serde_yaml::Value::String("daytona".into()),
            serde_yaml::Value::Mapping(daytona),
        );
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let runner = AgentRunner::new(&config);
        assert!(runner.is_err());
    }
}
