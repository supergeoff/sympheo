pub mod daytona;
pub mod local;

use crate::agent::parser::{AgentEvent, TurnResult};
use crate::error::SympheoError;
use crate::tracker::model::Issue;
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::mpsc::Receiver;

#[async_trait]
pub trait AgentBackend: Send + Sync {
    async fn run_turn(
        &self,
        issue: &Issue,
        prompt: &str,
        session_id: Option<&str>,
        workspace_path: &Path,
        cancelled: Arc<AtomicBool>,
    ) -> Result<(TurnResult, Receiver<AgentEvent>), SympheoError>;

    async fn cleanup_workspace(&self, _workspace_path: &Path) -> Result<(), SympheoError> {
        Ok(())
    }
}
