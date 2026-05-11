//! SPEC §10 CLI Adapter Contract.
//!
//! This module declares the language-neutral CLI adapter contract Sympheo expects from a
//! coding-agent CLI binary. The reference adapter is OpenCode (`opencode run`); other
//! adapters (e.g. `claude`, `pi`) MUST satisfy the same contract.
//!
//! Selection (§10.1): the orchestrator inspects the **leading binary token** of
//! `cli.command` and selects the first adapter whose claimed binaries match. If no
//! adapter matches, dispatch validation fails with [`SympheoError::CliAdapterNotFound`].
//!
//! Lifecycle (§10.2) lives on this trait. The execution surface (subprocess,
//! scriptable mock) is provided by an
//! [`crate::agent::backend::AgentBackend`] and is intentionally distinct from
//! the protocol the adapter speaks with the CLI binary.
//!
//! Options (§10.6): `cli.options` is a typed triplet shared across every
//! production adapter — see [`CliOptions`]. Each adapter projects the typed
//! fields onto its native flag set in [`CliAdapter::build_command_string`].

pub mod claude;
pub mod mock;
pub mod opencode;
pub mod pi;

use crate::agent::backend::AgentBackend;
use crate::agent::parser::{AgentEvent, EmittedEvent, TurnResult, parse_event_line};
use crate::config::typed::ServiceConfig;
use crate::error::SympheoError;
use crate::tracker::model::Issue;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::mpsc::Sender;

/// SPEC §10.6: shell-escape a string so it survives a `bash -lc` invocation
/// verbatim. Backslash-escapes shell metacharacters; not a full POSIX
/// quoter — but adequate for the controlled paths and identifiers adapters
/// pass through (`prompt_path`, `workspace_path`, opaque `session_id`).
pub(crate) fn shell_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`")
        .replace('\'', "\\'")
        .replace(';', "\\;")
        .replace('|', "\\|")
        .replace('&', "\\&")
        .replace('<', "\\<")
        .replace('>', "\\>")
        .replace('(', "\\(")
        .replace(')', "\\)")
        .replace('*', "\\*")
        .replace('?', "\\?")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('\n', "\\n")
}

/// Shared 8-4-4-4-12 hex UUID-shape check. Both `claude --resume` and
/// `opencode --session` reject non-UUID values; pi accepts a UUID *prefix*
/// but never invents one. Each adapter calls this helper before splicing
/// the session id into its argv.
pub(crate) fn is_uuid(s: &str) -> bool {
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

/// SPEC §10.1: leading-binary check shared by every adapter's `validate`.
/// Inspects the first whitespace-separated token of `cli_command` (stripping
/// any directory prefix), and verifies it matches one of the adapter's
/// claimed binary names.
pub(crate) fn validate_command_binary(
    cli_command: &str,
    binary_names: &[&'static str],
) -> Result<(), SympheoError> {
    if cli_command.trim().is_empty() {
        return Err(SympheoError::InvalidConfiguration(
            "cli.command is empty".into(),
        ));
    }
    let leading = cli_command.split_whitespace().next().unwrap_or("");
    let bin = Path::new(leading)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(leading);
    if !binary_names.contains(&bin) {
        return Err(SympheoError::CliAdapterNotFound(cli_command.to_string()));
    }
    Ok(())
}

/// SPEC §10.6: agent permission mode, shared across production adapters. Each
/// adapter projects this variant onto its native flag (see
/// `CliAdapter::build_command_string`). Variants mirror the four modes the
/// claude CLI accepts; adapters with no native equivalent (opencode, pi)
/// ignore the value and emit a structured warn on construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Plan,
    AcceptEdits,
    BypassPermissions,
    Default,
}

impl Permission {
    /// Canonical string form, also the value claude expects under
    /// `--permission-mode`.
    pub fn as_str(self) -> &'static str {
        match self {
            Permission::Plan => "plan",
            Permission::AcceptEdits => "acceptEdits",
            Permission::BypassPermissions => "bypassPermissions",
            Permission::Default => "default",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "plan" => Some(Permission::Plan),
            "acceptEdits" => Some(Permission::AcceptEdits),
            "bypassPermissions" => Some(Permission::BypassPermissions),
            "default" => Some(Permission::Default),
            _ => None,
        }
    }
}

/// SPEC §10.6: the typed triplet that every production adapter consumes.
/// `model` and `permission` are scalar overrides; `additional_args` is a
/// verbatim tail appended to the assembled argv (shell-escape applied
/// per-token). Unknown keys in the source map are silently ignored from the
/// typed view but remain available to adapter-specific extras (e.g. mock's
/// `script`).
///
/// Legacy keys (`permission_mode`, `permissions`, `mcp_servers`) are rejected
/// with a parse error pointing to the rename.
#[derive(Debug, Clone, Default)]
pub struct CliOptions {
    pub model: Option<String>,
    pub permission: Option<Permission>,
    pub additional_args: Vec<String>,
}

impl CliOptions {
    /// Parse the typed view from a raw `cli.options` map. Returns an empty
    /// instance if `value` is not an object. Errors on banned legacy keys.
    pub fn parse(value: &serde_json::Value) -> Result<Self, SympheoError> {
        let Some(map) = value.as_object() else {
            return Ok(Self::default());
        };

        if map.contains_key("permission_mode") {
            return Err(SympheoError::InvalidConfiguration(
                "cli.options.permission_mode is renamed to cli.options.permission".into(),
            ));
        }
        if map.contains_key("permissions") {
            return Err(SympheoError::InvalidConfiguration(
                "cli.options.permissions is renamed to cli.options.permission (singular)".into(),
            ));
        }
        if map.contains_key("mcp_servers") {
            return Err(SympheoError::InvalidConfiguration(
                "cli.options.mcp_servers is no longer supported; declare MCP servers via the agent's own config file".into(),
            ));
        }

        let model = map
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let permission = match map.get("permission") {
            None => None,
            Some(serde_json::Value::String(s)) => Some(Permission::parse(s).ok_or_else(|| {
                SympheoError::InvalidConfiguration(format!(
                    "cli.options.permission: invalid value '{s}' (expected one of: plan, acceptEdits, bypassPermissions, default)"
                ))
            })?),
            Some(_) => {
                return Err(SympheoError::InvalidConfiguration(
                    "cli.options.permission: expected a string (one of: plan, acceptEdits, bypassPermissions, default)"
                        .into(),
                ));
            }
        };

        let additional_args = match map.get("additional_args") {
            None => Vec::new(),
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str())
                .map(crate::config::resolver::resolve_value)
                .collect(),
            Some(_) => {
                return Err(SympheoError::InvalidConfiguration(
                    "cli.options.additional_args: expected an array of strings".into(),
                ));
            }
        };

        Ok(Self {
            model,
            permission,
            additional_args,
        })
    }

    /// Shallow merge: every field set in `override_` replaces the
    /// corresponding field in `self`. `additional_args` is treated as a single
    /// field — a non-empty override REPLACES the base entirely (not
    /// concatenated). Used to layer per-phase `cli.options` over the global
    /// `cli.options`.
    pub fn merge_over(&self, override_: &CliOptions) -> CliOptions {
        CliOptions {
            model: override_.model.clone().or_else(|| self.model.clone()),
            permission: override_.permission.or(self.permission),
            additional_args: if override_.additional_args.is_empty() {
                self.additional_args.clone()
            } else {
                override_.additional_args.clone()
            },
        }
    }
}

/// Append `--<flag> <value>` to a shell-string, shell-escaping the value.
pub(crate) fn append_flag(cmd: &mut String, flag: &str, value: &str) {
    cmd.push(' ');
    cmd.push_str(flag);
    cmd.push(' ');
    cmd.push_str(&shell_escape(value));
}

/// Append a verbatim tail of additional args, each shell-escaped.
pub(crate) fn append_additional_args(cmd: &mut String, args: &[String]) {
    for a in args {
        cmd.push(' ');
        cmd.push_str(&shell_escape(a));
    }
}

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
    pub env: HashMap<String, String>,
    pub options: CliOptions,
    /// SPEC §10.2.2: per-stdout-read stall timeout (default 5000 ms).
    pub read_timeout_ms: u64,
    /// SPEC §10.2.2: total wall-clock per turn (default 3600000 ms).
    pub turn_timeout_ms: u64,
}

impl CliConfig {
    pub fn from_service(config: &ServiceConfig) -> Result<Self, SympheoError> {
        let raw_options = config.cli_options_raw();
        let options = CliOptions::parse(&raw_options)?;
        Ok(Self {
            command: config.cli_command(),
            env: config.cli_env(),
            options,
            read_timeout_ms: config.cli_read_timeout_ms(),
            turn_timeout_ms: config.cli_turn_timeout_ms(),
        })
    }

    /// Build a per-turn variant where the global `options` are overridden by
    /// `phase_options` (shallow merge). Used by the orchestrator to project
    /// `phases[<active>].cli.options` over the dispatch-time config.
    pub fn with_effective_options(&self, phase_options: &CliOptions) -> Self {
        Self {
            command: self.command.clone(),
            env: self.env.clone(),
            options: self.options.merge_over(phase_options),
            read_timeout_ms: self.read_timeout_ms,
            turn_timeout_ms: self.turn_timeout_ms,
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

    /// SPEC §10.1: static configuration check — no network calls. Default
    /// implementation accepts any command whose leading binary token matches
    /// one of `binary_names()`. Adapters that need bespoke validation
    /// (subcommand checks, etc.) override.
    fn validate(&self, cli_command: &str) -> Result<(), SympheoError> {
        validate_command_binary(cli_command, self.binary_names())
    }

    /// SPEC §10.2.1: one-time setup per worker run. Default implementation
    /// allocates a Sympheo-side session id; adapters that need a CLI-managed
    /// identifier (or stateful setup) override.
    async fn start_session(
        &self,
        _workspace_path: &Path,
        _cli_config: &CliConfig,
    ) -> Result<SessionContext, SympheoError> {
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
        cli_config: &CliConfig,
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
                &cli_config.options,
            )
            .await
    }

    /// SPEC §10.2.3: final teardown. Default no-op (matches OpenCode where each
    /// turn is its own process). MUST be safe to call after a `run_turn` failure.
    async fn stop_session(&self, _session_context: &SessionContext) -> Result<(), SympheoError> {
        Ok(())
    }

    /// SPEC §10.6: build the shell command `LocalBackend` will spawn for this
    /// adapter. Default produces the OpenCode-shaped invocation; CLIs with
    /// different flag conventions (e.g. `claude --print`, `pi --mode json`)
    /// override.
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
        if let Some(sid) = session_id.filter(|s| is_uuid(s)) {
            append_flag(&mut cmd, "--session", sid);
        }
        append_additional_args(&mut cmd, &cli_options.additional_args);
        cmd
    }

    /// SPEC §10.6: parse one line of CLI stdout into a normalized
    /// [`AgentEvent`]. Default delegates to the OpenCode-shaped
    /// [`parse_event_line`].
    fn parse_stdout_line(&self, line: &str) -> Option<AgentEvent> {
        parse_event_line(line)
    }

    /// SPEC §10.6: optional pre-flight prompt sanitization. Default identity.
    /// OpenCode overrides this to escape lines that look like flags.
    fn sanitize_prompt(&self, prompt: &str) -> String {
        prompt.to_string()
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
        Arc::new(claude::ClaudeAdapter::new()),
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
    fn test_select_adapter_claude() {
        let adapter = select_adapter("claude").unwrap();
        assert_eq!(adapter.kind(), "claude");
        let adapter = select_adapter("claude --print").unwrap();
        assert_eq!(adapter.kind(), "claude");
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
    fn test_generate_session_id_starts_with_kind() {
        let id = generate_session_id("opencode");
        assert!(id.starts_with("opencode-"));
        assert!(id.len() > "opencode-".len());
    }

    #[test]
    fn test_shell_escape_backslash() {
        assert_eq!(shell_escape("a\\b"), "a\\\\b");
    }

    #[test]
    fn test_shell_escape_quote() {
        assert_eq!(shell_escape("say \"hi\""), "say \\\"hi\\\"");
    }

    #[test]
    fn test_shell_escape_dollar() {
        assert_eq!(shell_escape("$HOME"), "\\$HOME");
    }

    #[test]
    fn test_shell_escape_backtick() {
        assert_eq!(shell_escape("`cmd`"), "\\`cmd\\`");
    }

    #[test]
    fn test_shell_escape_combined() {
        assert_eq!(shell_escape("\\\"$`"), "\\\\\\\"\\$\\`");
    }

    #[test]
    fn test_is_uuid() {
        assert!(is_uuid("33595f82-f956-4338-854d-f6332a296842"));
        assert!(is_uuid("00000000-0000-0000-0000-000000000000"));
        assert!(!is_uuid(""));
        assert!(!is_uuid("claude-1234-5678"));
        assert!(!is_uuid("33595f82_f956_4338_854d_f6332a296842"));
        assert!(!is_uuid("33595f82-f956-4338-854d-f6332a296842-extra"));
        assert!(!is_uuid("zzzzzzzz-zzzz-zzzz-zzzz-zzzzzzzzzzzz"));
    }

    #[test]
    fn test_validate_command_binary_ok() {
        assert!(validate_command_binary("opencode run", &["opencode"]).is_ok());
        assert!(validate_command_binary("/usr/local/bin/opencode run", &["opencode"]).is_ok());
    }

    #[test]
    fn test_validate_command_binary_empty() {
        let err = validate_command_binary("", &["opencode"]).unwrap_err();
        assert!(matches!(err, SympheoError::InvalidConfiguration(_)));
    }

    #[test]
    fn test_validate_command_binary_wrong_bin() {
        let err = validate_command_binary("foo run", &["opencode"]).unwrap_err();
        assert!(matches!(err, SympheoError::CliAdapterNotFound(_)));
    }

    #[test]
    fn test_permission_roundtrip() {
        for p in [
            Permission::Plan,
            Permission::AcceptEdits,
            Permission::BypassPermissions,
            Permission::Default,
        ] {
            assert_eq!(Permission::parse(p.as_str()), Some(p));
        }
        assert_eq!(Permission::parse("bogus"), None);
    }

    #[test]
    fn test_cli_options_empty() {
        let opts = CliOptions::parse(&serde_json::Value::Null).unwrap();
        assert!(opts.model.is_none());
        assert!(opts.permission.is_none());
        assert!(opts.additional_args.is_empty());
    }

    #[test]
    fn test_cli_options_parse_typed() {
        let v = serde_json::json!({
            "model": "sonnet",
            "permission": "plan",
            "additional_args": ["--verbose", "--debug"]
        });
        let opts = CliOptions::parse(&v).unwrap();
        assert_eq!(opts.model.as_deref(), Some("sonnet"));
        assert_eq!(opts.permission, Some(Permission::Plan));
        assert_eq!(opts.additional_args, vec!["--verbose", "--debug"]);
    }

    #[test]
    fn test_cli_options_rejects_legacy_permission_mode() {
        let v = serde_json::json!({ "permission_mode": "plan" });
        let err = CliOptions::parse(&v).unwrap_err();
        assert!(
            matches!(err, SympheoError::InvalidConfiguration(s) if s.contains("permission_mode") && s.contains("renamed"))
        );
    }

    #[test]
    fn test_cli_options_rejects_legacy_permissions_plural() {
        let v = serde_json::json!({ "permissions": { "edit": true } });
        let err = CliOptions::parse(&v).unwrap_err();
        assert!(
            matches!(err, SympheoError::InvalidConfiguration(s) if s.contains("permissions") && s.contains("singular"))
        );
    }

    #[test]
    fn test_cli_options_rejects_legacy_mcp_servers() {
        let v = serde_json::json!({ "mcp_servers": {} });
        let err = CliOptions::parse(&v).unwrap_err();
        assert!(matches!(err, SympheoError::InvalidConfiguration(s) if s.contains("mcp_servers")));
    }

    #[test]
    fn test_cli_options_rejects_invalid_permission_value() {
        let v = serde_json::json!({ "permission": "yolo" });
        let err = CliOptions::parse(&v).unwrap_err();
        assert!(
            matches!(err, SympheoError::InvalidConfiguration(s) if s.contains("invalid value 'yolo'"))
        );
    }

    #[test]
    fn test_cli_options_rejects_non_array_additional_args() {
        let v = serde_json::json!({ "additional_args": "single" });
        let err = CliOptions::parse(&v).unwrap_err();
        assert!(
            matches!(err, SympheoError::InvalidConfiguration(s) if s.contains("additional_args"))
        );
    }

    #[test]
    fn test_cli_options_ignores_unknown_keys() {
        // mock's `script` lives outside the typed triplet — the parser must
        // not error on extras.
        let v = serde_json::json!({ "script": "fixtures/run.yaml" });
        let opts = CliOptions::parse(&v).unwrap();
        assert!(opts.model.is_none());
        assert!(opts.permission.is_none());
        assert!(opts.additional_args.is_empty());
    }

    #[test]
    fn test_cli_options_merge_override_wins() {
        let base = CliOptions {
            model: Some("sonnet".into()),
            permission: Some(Permission::Plan),
            additional_args: vec!["--global".into()],
        };
        let over = CliOptions {
            model: Some("haiku".into()),
            permission: None,
            additional_args: vec!["--phase".into()],
        };
        let merged = base.merge_over(&over);
        assert_eq!(merged.model.as_deref(), Some("haiku"));
        assert_eq!(merged.permission, Some(Permission::Plan));
        assert_eq!(merged.additional_args, vec!["--phase"]);
    }

    #[test]
    fn test_cli_options_merge_empty_override_keeps_base() {
        let base = CliOptions {
            model: Some("sonnet".into()),
            permission: Some(Permission::Plan),
            additional_args: vec!["--a".into(), "--b".into()],
        };
        let merged = base.merge_over(&CliOptions::default());
        assert_eq!(merged.model.as_deref(), Some("sonnet"));
        assert_eq!(merged.permission, Some(Permission::Plan));
        assert_eq!(merged.additional_args, vec!["--a", "--b"]);
    }
}
