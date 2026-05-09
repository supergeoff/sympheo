//! Mock CLI adapter — pairs with `crate::agent::backend::mock::MockBackend`.
//!
//! Selected when `cli.command` starts with the literal token `mock-cli`. Used
//! exclusively for tests and dry-runs; never for production traffic.

use crate::agent::cli::CliAdapter;
use crate::error::SympheoError;

pub struct MockCliAdapter;

impl MockCliAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MockCliAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl CliAdapter for MockCliAdapter {
    fn kind(&self) -> &str {
        "mock"
    }

    fn binary_names(&self) -> &[&'static str] {
        &["mock-cli"]
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
        if bin != "mock-cli" {
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
        let a = MockCliAdapter::new();
        assert_eq!(a.kind(), "mock");
        assert_eq!(a.binary_names(), &["mock-cli"]);
    }

    #[test]
    fn test_validate_mock_cli() {
        assert!(MockCliAdapter::new().validate("mock-cli").is_ok());
    }

    #[test]
    fn test_validate_mock_cli_with_args() {
        assert!(MockCliAdapter::new().validate("mock-cli --foo bar").is_ok());
    }

    #[test]
    fn test_validate_wrong_binary() {
        let err = MockCliAdapter::new().validate("opencode run").unwrap_err();
        assert!(matches!(err, SympheoError::CliAdapterNotFound(_)));
    }
}
