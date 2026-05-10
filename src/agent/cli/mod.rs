//! SPEC §10 CLI Adapter Contract.
//!
//! This module declares the language-neutral CLI adapter contract Sympheo expects from a
//! coding-agent CLI binary. The reference adapter is OpenCode (`opencode run`); other
//! adapters (e.g. `pi.dev`) MUST satisfy the same contract.
//!
//! Selection (§10.1): the orchestrator inspects the **leading binary token** of
//! `cli.command` and selects the first adapter whose claimed binaries match. If no
//! adapter matches, dispatch validation fails with [`SympheoError::CliAdapterNotFound`].
//!
//! Lifecycle (§10.2) lives on this trait. The execution surface (subprocess,
//! scriptable mock) is provided by an
//! [`crate::agent::backend::AgentBackend`] and is intentionally distinct from
//! the protocol the adapter speaks with the CLI binary.

pub mod mock;
pub mod opencode;
pub mod pi;

use crate::agent::backend::AgentBackend;
use crate::agent::parser::{EmittedEvent, TurnResult};
use crate::config::typed::ServiceConfig;
use crate::error::SympheoError;
use crate::tracker::model::Issue;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::mpsc::Sender;

/// SPEC §10.2.1: opaque session handle returned by `start_session` and threaded
/// through every `run_turn` / `stop_session` call within one worker run.
#[derive(Debug, Clone)]
pub struct SessionContext {
    /// Opaque CLI-managed identifier (e.g. OpenCode `--session <handle>`).
    pub agent_session_handle: String,
    /// Sympheo-side session id, unique per worker run.
    pub session_id: String,
}

/// SPEC §10.2.2 / §5.3.6: adapter-facing view of `cli.*` configuration. Built
/// from [`ServiceConfig`] at runner construction time so the adapter does not
/// need to reach back into the host config layer.
#[derive(Debug, Clone)]
pub struct CliConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub options: serde_json::Value,
    /// SPEC §10.2.2: per-stdout-read stall timeout (default 5000 ms).
    pub read_timeout_ms: u64,
    /// SPEC §10.2.2: total wall-clock per turn (default 3600000 ms).
    pub turn_timeout_ms: u64,
}

impl CliConfig {
    pub fn from_service(config: &ServiceConfig) -> Self {
        Self {
            command: config.cli_command(),
            args: config.cli_args(),
            env: config.cli_env(),
            options: config.cli_options(),
            read_timeout_ms: config.cli_read_timeout_ms(),
            turn_timeout_ms: config.cli_turn_timeout_ms(),
        }
    }
}

/// SPEC §10.1 + §10.2: a CLI adapter declares its identity, a static validation
/// routine, and the per-worker-run lifecycle (`start_session` / `run_turn` /
/// `stop_session`). The lifecycle owns the protocol the CLI speaks; the
/// execution surface is supplied by an
/// [`crate::agent::backend::AgentBackend`].
#[async_trait]
pub trait CliAdapter: Send + Sync {
    fn kind(&self) -> &str;
    fn binary_names(&self) -> &[&'static str];

    /// SPEC §10.1: static configuration check — no network calls.
    fn validate(&self, cli_command: &str) -> Result<(), SympheoError>;

    /// SPEC §10.6: keys the adapter recognizes inside `cli.options`. Anything
    /// outside this set is forwarded for forward-compatibility but emits a
    /// `tracing::warn!` from `start_session` so operators can detect typos.
    fn known_option_keys(&self) -> &[&'static str] {
        &[]
    }

    /// SPEC §10.2.1: one-time setup per worker run. Default implementation
    /// allocates a Sympheo-side session id and emits a warning for unknown
    /// `cli.options` keys; adapters that need a CLI-managed identifier
    /// (or stateful setup) override this.
    async fn start_session(
        &self,
        _workspace_path: &Path,
        cli_config: &CliConfig,
    ) -> Result<SessionContext, SympheoError> {
        warn_unknown_options(self.kind(), self.known_option_keys(), &cli_config.options);
        let id = generate_session_id(self.kind());
        Ok(SessionContext {
            agent_session_handle: id.clone(),
            session_id: id,
        })
    }

    /// SPEC §10.2.2: launch one CLI subprocess invocation for one turn. The
    /// adapter parses CLI output, forwards normalized events via `on_event`,
    /// and enforces both `cli.read_timeout_ms` and `cli.turn_timeout_ms`.
    /// Default implementation delegates to the supplied execution backend's
    /// `run_turn`, which still owns the CLI protocol for the reference
    /// adapters today; adapters that need bespoke protocol handling override.
    #[allow(clippy::too_many_arguments)] // Reason: SPEC §10.2.2 mandates this exact arity; bundling into a struct would just add indirection without changing the contract.
    async fn run_turn(
        &self,
        session_context: &SessionContext,
        prompt: &str,
        issue: &Issue,
        _turn_number: u32,
        cancelled: Arc<AtomicBool>,
        on_event: Sender<EmittedEvent>,
        executor: &dyn AgentBackend,
        _cli_config: &CliConfig,
        workspace_path: &Path,
    ) -> Result<TurnResult, SympheoError> {
        executor
            .run_turn(
                issue,
                prompt,
                Some(session_context.agent_session_handle.as_str()),
                workspace_path,
                cancelled,
                on_event,
            )
            .await
    }

    /// SPEC §10.2.3: final teardown. Default no-op (matches OpenCode where each
    /// turn is its own process). MUST be safe to call after a `run_turn` failure.
    async fn stop_session(&self, _session_context: &SessionContext) -> Result<(), SympheoError> {
        Ok(())
    }
}

/// SPEC §10.6: emit a `tracing::warn!` for each `cli.options` key the adapter
/// does not recognize. Unknown keys are not a fatal error (forward
/// compatibility) but operators need a signal to spot typos.
pub fn warn_unknown_options(
    adapter_kind: &str,
    known_keys: &[&'static str],
    options: &serde_json::Value,
) {
    let Some(map) = options.as_object() else {
        return;
    };
    for key in map.keys() {
        if !known_keys.contains(&key.as_str()) {
            tracing::warn!(
                adapter = %adapter_kind,
                option = %key,
                "unknown cli.options key (forwarded but not recognized; check for typos)"
            );
        }
    }
}

/// Generate an opaque, unique-per-call session id. Combines the adapter kind,
/// the host process id, and a nanosecond timestamp; collisions are practically
/// impossible within one worker process.
pub fn generate_session_id(adapter_kind: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{adapter_kind}-{}-{nanos}", std::process::id())
}

/// SPEC §10.1 + §5.5: select an adapter from the leading binary token of `cli.command`.
///
/// The leading token is taken to be the first whitespace-separated token after any
/// shell-style PATH lookup is stripped (we look at the file_name of the leading token
/// so that absolute paths like `/usr/local/bin/opencode run` still match `opencode`).
pub fn select_adapter(cli_command: &str) -> Result<Arc<dyn CliAdapter>, SympheoError> {
    let leading = cli_command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    if leading.is_empty() {
        return Err(SympheoError::CliAdapterNotFound(cli_command.to_string()));
    }
    let bin = Path::new(&leading)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&leading)
        .to_string();
    let candidates: Vec<Arc<dyn CliAdapter>> = vec![
        Arc::new(opencode::OpencodeAdapter::new()),
        Arc::new(pi::PiAdapter::new()),
        Arc::new(mock::MockCliAdapter::new()),
    ];
    for adapter in candidates {
        if adapter.binary_names().contains(&bin.as_str()) {
            adapter.validate(cli_command)?;
            return Ok(adapter);
        }
    }
    Err(SympheoError::CliAdapterNotFound(cli_command.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_adapter_opencode() {
        let adapter = select_adapter("opencode run").unwrap();
        assert_eq!(adapter.kind(), "opencode");
    }

    #[test]
    fn test_select_adapter_opencode_absolute_path() {
        let adapter = select_adapter("/usr/local/bin/opencode run").unwrap();
        assert_eq!(adapter.kind(), "opencode");
    }

    #[test]
    fn test_select_adapter_pi() {
        let adapter = select_adapter("pi run").unwrap();
        assert_eq!(adapter.kind(), "pidev");
    }

    #[test]
    fn test_select_adapter_unknown() {
        let result = select_adapter("foobar run");
        assert!(matches!(result, Err(SympheoError::CliAdapterNotFound(_))));
    }

    #[test]
    fn test_select_adapter_empty() {
        let result = select_adapter("");
        assert!(matches!(result, Err(SympheoError::CliAdapterNotFound(_))));
    }

    #[test]
    fn test_select_adapter_whitespace_only() {
        let result = select_adapter("   ");
        assert!(matches!(result, Err(SympheoError::CliAdapterNotFound(_))));
    }

    #[test]
    fn test_warn_unknown_options_does_not_panic_on_non_object() {
        warn_unknown_options("opencode", &["model"], &serde_json::Value::Null);
        warn_unknown_options(
            "opencode",
            &["model"],
            &serde_json::Value::String("scalar".into()),
        );
    }

    #[test]
    fn test_generate_session_id_starts_with_kind() {
        let id = generate_session_id("opencode");
        assert!(id.starts_with("opencode-"));
        assert!(id.len() > "opencode-".len());
    }
}
