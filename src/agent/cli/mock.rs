//! Mock CLI adapter — pairs with `crate::agent::backend::mock::MockBackend`.
//!
//! Selected when `cli.command` starts with the literal token `mock-cli`. Used
//! exclusively for tests and dry-runs; never for production traffic.
//!
//! Mock intentionally lives outside the shared `cli.options` triplet
//! (`model` / `permission` / `additional_args`) — it reads its scripted
//! event fixture from `cli.options.script` via the raw options accessor on
//! [`ServiceConfig`](crate::config::typed::ServiceConfig).

use crate::agent::cli::CliAdapter;
use async_trait::async_trait;

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

#[async_trait]
impl CliAdapter for MockCliAdapter {
    fn kind(&self) -> &str {
        "mock"
    }

    fn binary_names(&self) -> &[&'static str] {
        &["mock-cli"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SympheoError;

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
