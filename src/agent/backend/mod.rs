pub mod daytona;
pub mod local;

use async_trait::async_trait;
use std::path::Path;
use crate::tracker::model::Issue;
use crate::error::SympheoError;
use crate::agent::parser::TurnResult;

#[async_trait]
pub trait AgentBackend: Send + Sync {
    async fn run_turn(
        &self,
        issue: &Issue,
        prompt: &str,
        session_id: Option<&str>,
        workspace_path: &Path,
    ) -> Result<TurnResult, SympheoError>;
}
