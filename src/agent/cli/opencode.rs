//! SPEC §10.6 OpenCode Reference Adapter.
//!
//! `cli.command = "opencode run"` (default). The protocol-specific lifecycle
//! (`start_session` / `run_turn` / `stop_session`) is provided through the
//! [`crate::agent::cli::CliAdapter`] trait defaults, which delegate the
//! subprocess-spawning + stdout-parsing surface to the configured execution
//! [`crate::agent::backend::AgentBackend`] (`LocalBackend`, `MockBackend`).
//!
//! Permission projection: opencode does not currently expose a native
//! permission-mode flag analogous to claude's `--permission-mode`. Setting
//! `cli.options.permission` is therefore a no-op for this adapter (logged as
//! a warning so operators can spot the gap).

use crate::agent::cli::CliAdapter;
use crate::agent::cli::CliOptions;
use crate::agent::cli::append_additional_args;
use crate::agent::cli::append_flag;
use crate::agent::cli::is_uuid;
use crate::agent::cli::shell_escape;
use async_trait::async_trait;
use std::path::Path;

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

#[async_trait]
impl CliAdapter for OpencodeAdapter {
    fn kind(&self) -> &str {
        "opencode"
    }

    fn binary_names(&self) -> &[&'static str] {
        &["opencode"]
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
        cli_options: &CliOptions,
    ) -> String {
        let mut cmd = format!(
            r#"PROMPT=$(cat "{}"); {} "$PROMPT" --format json --dir "{}" --dangerously-skip-permissions"#,
            shell_escape(&prompt_path.to_string_lossy()),
            cli_command,
            shell_escape(&workspace_path.to_string_lossy())
        );
        if let Some(model) = &cli_options.model {
            append_flag(&mut cmd, "--model", model);
        }
        if let Some(permission) = cli_options.permission {
            // Opencode has no native --permission-mode equivalent today. We
            // record the intent in a tracing event so operators can spot
            // misconfiguration (and so the adapter remains a no-op
            // deliberately, not by oversight).
            tracing::warn!(
                adapter = "opencode",
                permission = permission.as_str(),
                "cli.options.permission is set but opencode has no native permission-mode flag; ignoring"
            );
        }
        if let Some(sid) = session_id.filter(|s| is_uuid(s)) {
            append_flag(&mut cmd, "--session", sid);
        }
        append_additional_args(&mut cmd, &cli_options.additional_args);
        cmd
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
    use crate::agent::cli::Permission;
    use crate::error::SympheoError;

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

    #[test]
    fn test_build_command_string_default() {
        let a = OpencodeAdapter::new();
        let prompt = Path::new("/ws/.sympheo_prompt.txt");
        let ws = Path::new("/ws");
        let cmd = a.build_command_string("opencode run", prompt, ws, None, &CliOptions::default());
        assert!(cmd.contains("opencode run"));
        assert!(cmd.contains("--format json"));
        assert!(cmd.contains("--dir"));
        assert!(cmd.contains("--dangerously-skip-permissions"));
        assert!(!cmd.contains("--session"));
        assert!(!cmd.contains("--model"));
    }

    #[test]
    fn test_build_command_string_splices_model_and_additional_args() {
        let a = OpencodeAdapter::new();
        let prompt = Path::new("/ws/.sympheo_prompt.txt");
        let ws = Path::new("/ws");
        let opts = CliOptions {
            model: Some("openrouter/anthropic/claude-haiku-4.5".into()),
            permission: None,
            additional_args: vec!["--print".into()],
        };
        let cmd = a.build_command_string("opencode run", prompt, ws, None, &opts);
        assert!(cmd.contains("--model openrouter/anthropic/claude-haiku-4.5"));
        assert!(cmd.contains("--print"));
    }

    #[test]
    fn test_build_command_string_permission_is_noop_for_opencode() {
        // Opencode does not currently expose a permission-mode flag; setting
        // permission must not splice anything into the command. The warning
        // is observable via tracing, not via the command string.
        let a = OpencodeAdapter::new();
        let prompt = Path::new("/ws/.sympheo_prompt.txt");
        let ws = Path::new("/ws");
        let opts = CliOptions {
            permission: Some(Permission::Plan),
            ..Default::default()
        };
        let cmd = a.build_command_string("opencode run", prompt, ws, None, &opts);
        assert!(!cmd.contains("--permission"));
        assert!(!cmd.contains("plan"));
    }

    #[test]
    fn test_build_command_string_with_uuid_session() {
        let a = OpencodeAdapter::new();
        let prompt = Path::new("/ws/.sympheo_prompt.txt");
        let ws = Path::new("/ws");
        let uuid = "33595f82-f956-4338-854d-f6332a296842";
        let cmd = a.build_command_string(
            "opencode run",
            prompt,
            ws,
            Some(uuid),
            &CliOptions::default(),
        );
        assert!(cmd.contains(&format!("--session {uuid}")));
    }
}
