//! SPEC §10.1 stub: pi.dev adapter.
//!
//! Selection identity only (no lifecycle yet). Reserved for the pi.dev CLI integration;
//! `validate()` accepts the shape `pi <subcommand>` but the orchestrator will reject any
//! attempt to actually run a turn until the lifecycle is implemented in a follow-up phase.

use crate::agent::cli::CliAdapter;
use crate::error::SympheoError;
use async_trait::async_trait;

pub struct PiAdapter;

impl PiAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PiAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CliAdapter for PiAdapter {
    fn kind(&self) -> &str {
        "pidev"
    }

    fn binary_names(&self) -> &[&'static str] {
        &["pi"]
    }

    fn validate(&self, cli_command: &str) -> Result<(), SympheoError> {
        if cli_command.trim().is_empty() {
            return Err(SympheoError::InvalidConfiguration(
                "cli.command is empty".into(),
            ));
        }
        let leading = cli_command.split_whitespace().next().unwrap_or("");
        let bin = std::path::Path::new(leading)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(leading);
        if bin != "pi" {
            return Err(SympheoError::CliAdapterNotFound(cli_command.to_string()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kind_and_names() {
        let a = PiAdapter::new();
        assert_eq!(a.kind(), "pidev");
        assert_eq!(a.binary_names(), &["pi"]);
    }

    #[test]
    fn test_validate_pi_run() {
        assert!(PiAdapter::new().validate("pi run").is_ok());
    }

    #[test]
    fn test_validate_wrong_binary() {
        let err = PiAdapter::new().validate("opencode run").unwrap_err();
        assert!(matches!(err, SympheoError::CliAdapterNotFound(_)));
    }

    #[test]
    fn test_validate_empty() {
        let err = PiAdapter::new().validate("").unwrap_err();
        assert!(matches!(err, SympheoError::InvalidConfiguration(_)));
    }
}
