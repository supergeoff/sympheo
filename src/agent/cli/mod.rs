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
//! Note (P1 scope): only the **identity / selection / validate** surface of §10.1 is
//! migrated here. The full lifecycle separation (`start_session` / `run_turn` /
//! `stop_session` per §10.2) remains co-located with the executor backends
//! (`LocalBackend`, `DaytonaBackend`) and will be migrated in a follow-up phase.

pub mod opencode;
pub mod pi;

use crate::error::SympheoError;
use std::path::Path;
use std::sync::Arc;

/// SPEC §10.1: a CLI adapter declares its `kind`, the binary names it claims,
/// and a static validation routine.
pub trait CliAdapter: Send + Sync {
    fn kind(&self) -> &str;
    fn binary_names(&self) -> &[&'static str];
    /// Static configuration check — no network calls.
    fn validate(&self, cli_command: &str) -> Result<(), SympheoError>;
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
}
