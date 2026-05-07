use async_trait::async_trait;
use std::path::Path;
use crate::tracker::model::Issue;
use crate::error::SymphonyError;
use crate::agent::parser::TurnResult;

#[async_trait]
pub trait AgentBackend: Send + Sync {
    async fn run_turn(
        &self,
        issue: &Issue,
        prompt: &str,
        session_id: Option<&str>,
        workspace_path: &Path,
    ) -> Result<TurnResult, SymphonyError>;
}
