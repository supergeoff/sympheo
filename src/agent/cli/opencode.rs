//! SPEC §10.6 OpenCode Reference Adapter.
//!
//! This is the reference adapter: `cli.command = "opencode run"` (default).
//! Identity/selection/validate live here; the protocol-specific lifecycle
//! (`start_session` / `run_turn` / `stop_session`) is provided through the
//! [`crate::agent::cli::CliAdapter`] trait defaults, which delegate the
//! subprocess-spawning + stdout-parsing surface to the configured execution
//! [`crate::agent::backend::AgentBackend`] (`LocalBackend`, `MockBackend`).

use crate::agent::cli::CliAdapter;
use crate::agent::cli::shell_escape;
use crate::error::SympheoError;
use async_trait::async_trait;
use std::path::Path;

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

    /// `opencode run --session <id>` expects a UUID-shaped opencode session id
    /// that already exists on disk. Sympheo's default `start_session` allocates
    /// a synthetic `opencode-<pid>-<ts>` handle which opencode silently rejects
    /// (the turn exits without emitting a stop-shaped `step_finish`, so the
    /// backend classifies it as `Failed`). Until opencode's own session UUID is
    /// captured from the `session.created` event and threaded back through, we
    /// only emit `--session` when the caller's value already looks like a UUID.
    fn build_command_string(
        &self,
        cli_command: &str,
        prompt_path: &Path,
        workspace_path: &Path,
        session_id: Option<&str>,
    ) -> String {
        let mut cmd = format!(
            r#"PROMPT=$(cat "{}"); {} "$PROMPT" --format json --dir "{}" --dangerously-skip-permissions"#,
            shell_escape(&prompt_path.to_string_lossy()),
            cli_command,
            shell_escape(&workspace_path.to_string_lossy())
        );
        if let Some(sid) = session_id.filter(|s| is_uuid(s)) {
            cmd.push_str(&format!(" --session {}", shell_escape(sid)));
        }
        cmd
    }
}

/// Tightly-scoped UUID-shape check (8-4-4-4-12 hex). Mirrors the helper in
/// `claude.rs`: both adapters wrap a synthetic non-UUID handle from sympheo's
/// `start_session` and need to skip the per-turn `--resume`/`--session` flag
/// when the value doesn't match the CLI's expected shape.
fn is_uuid(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (i, &b) in bytes.iter().enumerate() {
        match i {
            8 | 13 | 18 | 23 => {
                if b != b'-' {
                    return false;
                }
            }
            _ => {
                if !b.is_ascii_hexdigit() {
                    return false;
                }
            }
        }
    }
    true
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
