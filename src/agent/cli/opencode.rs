//! SPEC §10.6 OpenCode Reference Adapter.
//!
//! This is the reference adapter: `cli.command = "opencode run"` (default).
//! For now, only the identity/selection/validate surface is implemented here;
//! the per-turn lifecycle (`start_session` / `run_turn` / `stop_session`) lives
//! in `crate::agent::backend::local`.

use crate::agent::cli::CliAdapter;
use crate::error::SympheoError;

/// Tested OpenCode CLI version range (advisory; not enforced at runtime).
/// SPEC §10.6 RECOMMENDED: adapters MUST document the CLI version range they support.
pub const SUPPORTED_OPENCODE_VERSION_RANGE: &str = ">=0.1, <0.5";

pub struct OpencodeAdapter;

impl OpencodeAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for OpencodeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl CliAdapter for OpencodeAdapter {
    fn kind(&self) -> &str {
        "opencode"
    }

    fn binary_names(&self) -> &[&'static str] {
        &["opencode"]
    }

    /// SPEC §10.1 + §10.6: static validation of the CLI command.
    /// We only verify shape; the binary's PATH discoverability is checked at runtime.
    fn validate(&self, cli_command: &str) -> Result<(), SympheoError> {
        if cli_command.trim().is_empty() {
            return Err(SympheoError::InvalidConfiguration(
                "cli.command is empty".into(),
            ));
        }
        // SPEC §10.6 default: "opencode run". Any "opencode <subcommand>" form is accepted.
        let leading = cli_command.split_whitespace().next().unwrap_or("");
        let bin = std::path::Path::new(leading)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(leading);
        if bin != "opencode" {
            return Err(SympheoError::CliAdapterNotFound(cli_command.to_string()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kind() {
        assert_eq!(OpencodeAdapter::new().kind(), "opencode");
    }

    #[test]
    fn test_binary_names() {
        assert_eq!(OpencodeAdapter::new().binary_names(), &["opencode"]);
    }

    #[test]
    fn test_validate_default_command() {
        assert!(OpencodeAdapter::new().validate("opencode run").is_ok());
    }

    #[test]
    fn test_validate_absolute_path() {
        assert!(
            OpencodeAdapter::new()
                .validate("/usr/local/bin/opencode run")
                .is_ok()
        );
    }

    #[test]
    fn test_validate_empty() {
        let err = OpencodeAdapter::new().validate("").unwrap_err();
        assert!(matches!(err, SympheoError::InvalidConfiguration(_)));
    }

    #[test]
    fn test_validate_wrong_binary() {
        let err = OpencodeAdapter::new().validate("foo run").unwrap_err();
        assert!(matches!(err, SympheoError::CliAdapterNotFound(_)));
    }

    #[test]
    fn test_supported_version_range_is_set() {
        assert!(!SUPPORTED_OPENCODE_VERSION_RANGE.is_empty());
    }
}
