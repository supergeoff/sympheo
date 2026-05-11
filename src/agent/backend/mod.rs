pub mod local;
pub mod mock;

use crate::agent::cli::CliOptions;
use crate::agent::parser::{EmittedEvent, TurnResult};
use crate::error::SympheoError;
use crate::tracker::model::Issue;
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::mpsc::Sender;

/// SPEC §10 — execution surface for one agent turn.
///
/// Distinct from [`crate::agent::cli::CliAdapter`]: the **adapter** owns the
/// CLI protocol (per-turn lifecycle, options validation, prompt continuation
/// semantics), while the **backend** owns the execution surface (subprocess
/// vs. scriptable mock). For the OpenCode reference adapter the backend still
/// drives the on-the-wire protocol concretely; `run_turn` is wrapped through
/// the adapter trait's default implementation so adapter-level tests can
/// verify the lifecycle independently.
#[async_trait]
pub trait AgentBackend: Send + Sync {
    /// Identifier of the execution surface (e.g. `local`, `mock`).
    fn kind(&self) -> &'static str;

    /// Run a single agent turn.
    ///
    /// The backend pushes [`EmittedEvent`]s (parsed event + active turn PID
    /// per SPEC §10.3) into `event_tx` as they arrive, so the orchestrator
    /// (which owns the receiver) can update live state in real time instead
    /// of waiting for the turn to finish. The sender is consumed by
    /// ownership: when this function returns, all clones must have been
    /// dropped so the consumer task can drain and exit.
    #[allow(clippy::too_many_arguments)] // Reason: SPEC §10.2.2 mandates the exact arity; bundling into a struct would add indirection without changing the contract.
    async fn run_turn(
        &self,
        issue: &Issue,
        prompt: &str,
        session_id: Option<&str>,
        workspace_path: &Path,
        cancelled: Arc<AtomicBool>,
        event_tx: Sender<EmittedEvent>,
        cli_options: &CliOptions,
    ) -> Result<TurnResult, SympheoError>;

    async fn cleanup_workspace(&self, _workspace_path: &Path) -> Result<(), SympheoError> {
        Ok(())
    }
}
