//! SPEC §10.6 OpenCode Reference Adapter.
//!
//! This is the reference adapter: `cli.command = "opencode run"` (default).
//! Identity/selection/validate live here; the protocol-specific lifecycle
//! (`start_session` / `run_turn` / `stop_session`) is provided through the
//! [`crate::agent::cli::CliAdapter`] trait defaults, which delegate the
//! subprocess-spawning + stdout-parsing surface to the configured execution
//! [`crate::agent::backend::AgentBackend`] (`LocalBackend`, `MockBackend`).

use crate::agent::cli::CliAdapter;
use crate::error::SympheoError;
use async_trait::async_trait;

/// Tested OpenCode CLI version range (advisory; not enforced at runtime).
/// SPEC §10.6 RECOMMENDED: adapters MUST document the CLI version range they support.
pub const SUPPORTED_OPENCODE_VERSION_RANGE: &str = ">=0.1, <0.5";

/// SPEC §10.6: keys the OpenCode adapter recognizes inside `cli.options`. Any
/// other key is forwarded for forward-compatibility and logged as a warning
/// from `start_session`.
pub const OPENCODE_KNOWN_OPTION_KEYS: &[&str] = &["model", "permissions", "mcp_servers"];

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

#[async_trait]
impl CliAdapter for OpencodeAdapter {
    fn kind(&self) -> &str {
        "opencode"
    }

    fn binary_names(&self) -> &[&'static str] {
        &["opencode"]
    }

    fn known_option_keys(&self) -> &[&'static str] {
        OPENCODE_KNOWN_OPTION_KEYS
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

    /// SPEC §10.6: opencode mistakes prompt lines that look like CLI flags
    /// (`^--foo$`) for actual flag arguments. Wrap such lines in backticks so
    /// the CLI treats them as literal prompt content.
    fn sanitize_prompt(&self, prompt: &str) -> String {
        sanitize_prompt_for_opencode(prompt)
    }
}

/// SPEC §10.6: lines matching `^--[a-z0-9-]+$` are wrapped in backticks so
/// opencode does not interpret them as flag arguments. Public within the
/// crate so the OpenCode adapter test surface can exercise it directly.
fn sanitize_prompt_for_opencode(prompt: &str) -> String {
    let re = regex::Regex::new(r"(?m)^--[a-z0-9-]+$").unwrap();
    re.replace_all(prompt, |caps: &regex::Captures| format!("`{}`", &caps[0]))
        .to_string()
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

    #[test]
    fn test_known_option_keys_documented() {
        let a = OpencodeAdapter::new();
        let keys = a.known_option_keys();
        assert!(keys.contains(&"model"));
        assert!(keys.contains(&"permissions"));
        assert!(keys.contains(&"mcp_servers"));
    }

    #[test]
    fn test_sanitize_prompt_wraps_flag_like_lines() {
        let a = OpencodeAdapter::new();
        let raw = "hello\n--foo-bar\nworld";
        let out = a.sanitize_prompt(raw);
        assert!(out.contains("`--foo-bar`"));
        assert!(out.contains("hello"));
        assert!(out.contains("world"));
    }

    #[test]
    fn test_sanitize_prompt_leaves_non_flag_lines() {
        let a = OpencodeAdapter::new();
        let raw = "no flags here";
        assert_eq!(a.sanitize_prompt(raw), raw);
    }
}
