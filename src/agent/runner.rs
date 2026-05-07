use crate::agent::backend::{daytona::DaytonaBackend, local::LocalBackend};
use crate::agent::backend::AgentBackend;
use crate::agent::parser::TurnResult;
use crate::config::typed::ServiceConfig;
use crate::error::SymphonyError;
use crate::tracker::model::Issue;
use std::path::Path;
use std::sync::Arc;

pub struct AgentRunner {
    backend: Arc<dyn AgentBackend>,
}

impl AgentRunner {
    pub fn new(config: &ServiceConfig) -> Result<Self, SymphonyError> {
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
    ) -> Result<TurnResult, SymphonyError> {
        self.backend.run_turn(issue, prompt, session_id, workspace_path).await
    }
}
