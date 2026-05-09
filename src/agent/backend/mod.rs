pub mod daytona;
pub mod local;
pub mod mock;

use crate::agent::parser::{AgentEvent, TurnResult};
use crate::error::SympheoError;
use crate::tracker::model::Issue;
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::mpsc::Sender;

#[async_trait]
pub trait AgentBackend: Send + Sync {
    /// Run a single agent turn.
    ///
    /// The backend pushes parsed events into `event_tx` as they arrive, so the
    /// orchestrator (which owns the receiver) can update live state in real
    /// time instead of waiting for the turn to finish. The sender is consumed
    /// by ownership: when this function returns, all clones must have been
    /// dropped so the consumer task can drain and exit.
    async fn run_turn(
        &self,
        issue: &Issue,
        prompt: &str,
        session_id: Option<&str>,
        workspace_path: &Path,
        cancelled: Arc<AtomicBool>,
        event_tx: Sender<AgentEvent>,
    ) -> Result<TurnResult, SympheoError>;

    async fn cleanup_workspace(&self, _workspace_path: &Path) -> Result<(), SympheoError> {
        Ok(())
    }
}
