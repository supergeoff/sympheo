use crate::agent::backend::AgentBackend;
use crate::agent::backend::{local::LocalBackend, mock::MockBackend};
use crate::agent::cli::{CliAdapter, CliConfig, CliOptions, SessionContext, select_adapter};
use crate::agent::parser::{EmittedEvent, TurnResult};
use crate::config::typed::ServiceConfig;
use crate::error::SympheoError;
use crate::tracker::model::Issue;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::mpsc::Sender;

pub struct AgentRunner {
    adapter: Arc<dyn CliAdapter>,
    backend: Arc<dyn AgentBackend>,
    cli_config: CliConfig,
}

impl AgentRunner {
    pub fn new(config: &ServiceConfig) -> Result<Self, SympheoError> {
        let cli_config = CliConfig::from_service(config)?;
        let adapter = select_adapter(&cli_config.command)?;
        let leading = cli_config
            .command
            .split_whitespace()
            .next()
            .map(|s| {
                std::path::Path::new(s)
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or(s)
                    .to_string()
            })
            .unwrap_or_default();
        let backend: Arc<dyn AgentBackend> = if leading == "mock-cli" {
            // P5 mock adapter for tests / dry-runs (zero tokens). Selected by
            // `cli.command = "mock-cli"`. Reads a YAML/JSON event script from
            // cli.options.script and replays it.
            Arc::new(MockBackend::new(config)?)
        } else {
            Arc::new(LocalBackend::new(config, adapter.clone())?)
        };
        Ok(Self {
            adapter,
            backend,
            cli_config,
        })
    }

    pub fn adapter_kind(&self) -> &str {
        self.adapter.kind()
    }

    pub fn backend_kind(&self) -> &'static str {
        self.backend.kind()
    }

    /// SPEC §10.2.1: one-time per-worker-run setup.
    pub async fn start_session(
        &self,
        workspace_path: &Path,
    ) -> Result<SessionContext, SympheoError> {
        self.adapter
            .start_session(workspace_path, &self.cli_config)
            .await
    }

    /// SPEC §10.2.2: one CLI subprocess invocation for one turn.
    ///
    /// `phase_options` is the per-phase override map looked up from
    /// `phases[<active>].cli.options` and shallow-merged over the global
    /// `cli.options` at dispatch time. Pass `None` when no phase is active
    /// (continuation turn outside a phased workflow) — the global options
    /// apply verbatim.
    #[allow(clippy::too_many_arguments)] // Reason: forwards verbatim to `CliAdapter::run_turn`, whose arity is fixed by SPEC §10.2.2; refactoring here would only diverge the two signatures.
    pub async fn run_turn(
        &self,
        session: &SessionContext,
        prompt: &str,
        issue: &Issue,
        turn_number: u32,
        workspace_path: &Path,
        cancelled: Arc<AtomicBool>,
        event_tx: Sender<EmittedEvent>,
        phase_options: Option<&CliOptions>,
    ) -> Result<TurnResult, SympheoError> {
        let effective_cli_config = match phase_options {
            Some(po) => self.cli_config.with_effective_options(po),
            None => self.cli_config.clone(),
        };
        self.adapter
            .run_turn(
                session,
                prompt,
                issue,
                turn_number,
                cancelled,
                event_tx,
                self.backend.as_ref(),
                &effective_cli_config,
                workspace_path,
            )
            .await
    }

    /// SPEC §10.2.3: per-worker-run teardown. Safe to call after a `run_turn`
    /// failure.
    pub async fn stop_session(&self, session: &SessionContext) -> Result<(), SympheoError> {
        self.adapter.stop_session(session).await
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
        workspace.insert("root".into(), serde_json::Value::String("/tmp".into()));
        raw.insert("workspace".into(), serde_json::Value::Object(workspace));
        raw
    }

    #[test]
    fn test_agent_runner_local_success() {
        let mut raw = base_config();
        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        cli.insert(
            "command".into(),
            serde_json::Value::String("opencode run".into()),
        );
        raw.insert("cli".into(), serde_json::Value::Object(cli));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let runner = AgentRunner::new(&config).unwrap();
        assert_eq!(runner.adapter_kind(), "opencode");
        assert_eq!(runner.backend_kind(), "local");
    }

    /// SPEC §17.6: adapter selection picks the right adapter regardless of
    /// what backend ends up running (selection is keyed off the leading
    /// binary token of `cli.command`).
    #[test]
    fn test_agent_runner_selects_mock_adapter() {
        let mut raw = base_config();
        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        cli.insert(
            "command".into(),
            serde_json::Value::String("mock-cli".into()),
        );
        let mut opts = serde_json::Map::<String, serde_json::Value>::new();
        opts.insert(
            "script".into(),
            serde_json::Value::String("script.yaml".into()),
        );
        cli.insert("options".into(), serde_json::Value::Object(opts));
        raw.insert("cli".into(), serde_json::Value::Object(cli));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let runner = AgentRunner::new(&config).unwrap();
        assert_eq!(runner.adapter_kind(), "mock");
        assert_eq!(runner.backend_kind(), "mock");
    }

    /// SPEC §10.2.1 + §10.2.3: the adapter lifecycle is callable
    /// independently of any actual turn — `start_session` / `stop_session`
    /// work for the reference adapters without touching the execution
    /// backend.
    #[tokio::test]
    async fn test_agent_runner_lifecycle_independent_of_backend() {
        let mut raw = base_config();
        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        cli.insert(
            "command".into(),
            serde_json::Value::String("opencode run".into()),
        );
        raw.insert("cli".into(), serde_json::Value::Object(cli));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let runner = AgentRunner::new(&config).unwrap();
        let ctx = runner
            .start_session(std::path::Path::new("/tmp"))
            .await
            .expect("start_session should succeed");
        assert!(!ctx.session_id.is_empty());
        assert!(!ctx.agent_session_handle.is_empty());
        runner
            .stop_session(&ctx)
            .await
            .expect("stop_session should succeed (default no-op)");
    }
}
