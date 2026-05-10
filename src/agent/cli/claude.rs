//! SPEC §10 Claude CLI adapter.
//!
//! Drives the official Anthropic `claude` CLI via
//! `--print --output-format stream-json --verbose`.
//!
//! Captures the session id from the `system.init` event so subsequent turns
//! can pass `--resume <sid>` for multi-turn continuation. The first turn omits
//! `--resume`; the orchestrator threads the captured id back in through
//! [`crate::agent::cli::SessionContext`] / `LocalBackend::run_turn`'s
//! `session_id` parameter.

use crate::agent::cli::{CliAdapter, shell_escape};
use crate::agent::parser::{
    AgentEvent, StepFinishPart, StepStartPart, TextPart, TextTime, TokenInfo,
};
use crate::error::SympheoError;
use async_trait::async_trait;
use std::path::Path;

/// Tested Claude CLI version range (advisory; not enforced at runtime).
/// SPEC §10.6 RECOMMENDED: adapters MUST document the CLI version range they support.
pub const SUPPORTED_CLAUDE_VERSION_RANGE: &str = ">=0.1";

/// SPEC §10.6: keys the Claude adapter recognizes inside `cli.options`. Any
/// other key is forwarded for forward-compatibility and logged as a warning
/// from `start_session`.
pub const CLAUDE_KNOWN_OPTION_KEYS: &[&str] = &["model", "permission_mode", "additional_args"];

pub struct ClaudeAdapter;

impl ClaudeAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ClaudeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CliAdapter for ClaudeAdapter {
    fn kind(&self) -> &str {
        "claude"
    }

    fn binary_names(&self) -> &[&'static str] {
        &["claude"]
    }

    fn known_option_keys(&self) -> &[&'static str] {
        CLAUDE_KNOWN_OPTION_KEYS
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
        if bin != "claude" {
            return Err(SympheoError::CliAdapterNotFound(cli_command.to_string()));
        }
        Ok(())
    }

    fn build_command_string(
        &self,
        cli_command: &str,
        prompt_path: &Path,
        workspace_path: &Path,
        session_id: Option<&str>,
    ) -> String {
        // Claude requires `--verbose` whenever `--output-format=stream-json`
        // is used together with `--print`. `--add-dir` widens the workspace
        // sandbox to include the per-issue checkout directory.
        let mut cmd = format!(
            r#"PROMPT=$(cat "{}"); {} --print "$PROMPT" --output-format stream-json --verbose --add-dir "{}" --dangerously-skip-permissions"#,
            shell_escape(&prompt_path.to_string_lossy()),
            cli_command,
            shell_escape(&workspace_path.to_string_lossy()),
        );
        // `claude --print --resume <id>` requires the id to be a UUID (or a
        // session title that already exists). Sympheo's default
        // `start_session` allocates a synthetic identifier of the shape
        // `claude-<pid>-<ts>` which the CLI rejects with
        //   `--resume requires a valid session ID or session title`.
        // The orchestrator does not feed the captured UUID back into the
        // adapter between turns, so for now we only emit `--resume` when the
        // caller provides a value that already looks like a UUID. First turn
        // (no continuation) always omits the flag.
        if let Some(sid) = session_id.filter(|s| is_uuid(s)) {
            cmd.push_str(&format!(" --resume {}", shell_escape(sid)));
        }
        cmd
    }

    fn parse_stdout_line(&self, line: &str) -> Option<AgentEvent> {
        // Claude `stream-json` emits one JSON object per line. We map the three
        // shapes the orchestrator's run-loop reacts to:
        //   {"type":"system","subtype":"init","session_id":"...","cwd":"...","model":"..."}
        //   {"type":"assistant","message":{"content":[{"type":"text","text":"..."}]},"session_id":"..."}
        //   {"type":"result","subtype":"success","session_id":"...","usage":{...},"is_error":false}
        // onto opencode-shaped `AgentEvent` variants so `LocalBackend::run_turn`
        // reacts uniformly across adapters.
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        let kind = v.get("type")?.as_str()?;
        let session_id = v
            .get("session_id")
            .and_then(|s| s.as_str())
            .unwrap_or("claude-session")
            .to_string();
        match kind {
            "system" => {
                let subtype = v.get("subtype").and_then(|s| s.as_str()).unwrap_or("");
                if subtype != "init" {
                    return Some(AgentEvent::Other);
                }
                Some(AgentEvent::StepStart {
                    timestamp: 0,
                    session_id: session_id.clone(),
                    part: StepStartPart {
                        id: format!("{session_id}-init"),
                        message_id: format!("{session_id}-init"),
                        session_id,
                        part_type: "system_init".into(),
                    },
                })
            }
            "assistant" => {
                let content = v.get("message")?.get("content")?.as_array()?;
                let mut text = String::new();
                for c in content {
                    if c.get("type").and_then(|t| t.as_str()) == Some("text")
                        && let Some(t) = c.get("text").and_then(|t| t.as_str())
                    {
                        text.push_str(t);
                    }
                }
                if text.is_empty() {
                    return Some(AgentEvent::Other);
                }
                Some(AgentEvent::Text {
                    timestamp: 0,
                    session_id: session_id.clone(),
                    part: TextPart {
                        id: format!("{session_id}-text"),
                        message_id: format!("{session_id}-msg"),
                        session_id,
                        part_type: "text".into(),
                        text,
                        time: Some(TextTime { start: 0, end: 0 }),
                    },
                })
            }
            "result" => {
                let subtype = v
                    .get("subtype")
                    .and_then(|s| s.as_str())
                    .unwrap_or("success");
                let is_error = v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false);
                let reason = if is_error || subtype.contains("error") {
                    "error".to_string()
                } else {
                    "stop".to_string()
                };
                let tokens = v.get("usage").map(|u| {
                    let input = u.get("input_tokens").and_then(|n| n.as_u64()).unwrap_or(0);
                    let output = u.get("output_tokens").and_then(|n| n.as_u64()).unwrap_or(0);
                    TokenInfo {
                        total: input + output,
                        input,
                        output,
                        reasoning: 0,
                        cache: None,
                    }
                });
                Some(AgentEvent::StepFinish {
                    timestamp: 0,
                    session_id: session_id.clone(),
                    part: StepFinishPart {
                        id: format!("{session_id}-finish"),
                        reason,
                        message_id: format!("{session_id}-msg"),
                        session_id,
                        part_type: "step_finish".into(),
                        tokens,
                        cost: v.get("total_cost_usd").and_then(|c| c.as_f64()),
                    },
                })
            }
            _ => Some(AgentEvent::Other),
        }
    }
}

/// Tightly-scoped UUID-shape check (8-4-4-4-12 hex). The claude CLI rejects
/// `--resume` values that are neither a valid UUID nor a saved session title;
/// rather than parse the full RFC, we accept the canonical 36-char hyphenated
/// form which is what claude emits in its `system.init` event.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kind_and_names() {
        let a = ClaudeAdapter::new();
        assert_eq!(a.kind(), "claude");
        assert_eq!(a.binary_names(), &["claude"]);
    }

    #[test]
    fn test_validate_positive() {
        let a = ClaudeAdapter::new();
        assert!(a.validate("claude").is_ok());
        assert!(a.validate("claude --print").is_ok());
        assert!(a.validate("/usr/local/bin/claude").is_ok());
    }

    #[test]
    fn test_validate_negative() {
        let a = ClaudeAdapter::new();
        let err = a.validate("").unwrap_err();
        assert!(matches!(err, SympheoError::InvalidConfiguration(_)));
        let err = a.validate("opencode run").unwrap_err();
        assert!(matches!(err, SympheoError::CliAdapterNotFound(_)));
    }

    #[test]
    fn test_supported_version_range_is_set() {
        assert!(!SUPPORTED_CLAUDE_VERSION_RANGE.is_empty());
    }

    #[test]
    fn test_known_option_keys_documented() {
        let a = ClaudeAdapter::new();
        let keys = a.known_option_keys();
        assert!(keys.contains(&"model"));
        assert!(keys.contains(&"permission_mode"));
        assert!(keys.contains(&"additional_args"));
    }

    #[test]
    fn test_build_command_string_first_turn() {
        let a = ClaudeAdapter::new();
        let prompt = Path::new("/ws/.sympheo_prompt.txt");
        let ws = Path::new("/ws");
        let cmd = a.build_command_string("claude", prompt, ws, None);
        assert!(cmd.contains("claude --print"));
        assert!(cmd.contains("--output-format stream-json"));
        assert!(cmd.contains("--verbose"));
        assert!(cmd.contains("--add-dir"));
        assert!(cmd.contains("--dangerously-skip-permissions"));
        assert!(
            !cmd.contains("--resume"),
            "first turn must not include --resume"
        );
    }

    #[test]
    fn test_build_command_string_with_uuid_session() {
        let a = ClaudeAdapter::new();
        let prompt = Path::new("/ws/.sympheo_prompt.txt");
        let ws = Path::new("/ws");
        let uuid = "33595f82-f956-4338-854d-f6332a296842";
        let cmd = a.build_command_string("claude", prompt, ws, Some(uuid));
        assert!(cmd.contains(&format!("--resume {uuid}")));
    }

    #[test]
    fn test_build_command_string_skips_resume_for_synthetic_id() {
        // SPEC §10.6: claude rejects --resume <non-uuid>; the orchestrator's
        // synthetic claude-PID-TS handle must NOT propagate to the CLI.
        let a = ClaudeAdapter::new();
        let prompt = Path::new("/ws/.sympheo_prompt.txt");
        let ws = Path::new("/ws");
        let cmd = a.build_command_string("claude", prompt, ws, Some("claude-1234-5678"));
        assert!(
            !cmd.contains("--resume"),
            "non-UUID handle must not produce --resume; cmd={cmd}"
        );
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
    fn test_parse_stdout_line_system_init() {
        let a = ClaudeAdapter::new();
        let line = r#"{"type":"system","subtype":"init","session_id":"sess-1","cwd":"/ws","model":"claude-3"}"#;
        let event = a.parse_stdout_line(line).expect("must parse");
        match event {
            AgentEvent::StepStart {
                session_id, part, ..
            } => {
                assert_eq!(session_id, "sess-1");
                assert_eq!(part.session_id, "sess-1");
                assert_eq!(part.part_type, "system_init");
            }
            other => panic!("expected StepStart, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_stdout_line_assistant_text() {
        let a = ClaudeAdapter::new();
        let line = r#"{"type":"assistant","session_id":"sess-1","message":{"content":[{"type":"text","text":"hello world"}]}}"#;
        let event = a.parse_stdout_line(line).expect("must parse");
        match event {
            AgentEvent::Text { part, .. } => {
                assert_eq!(part.text, "hello world");
                assert_eq!(part.session_id, "sess-1");
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_stdout_line_result_success() {
        let a = ClaudeAdapter::new();
        let line = r#"{"type":"result","subtype":"success","session_id":"sess-1","is_error":false,"usage":{"input_tokens":50,"output_tokens":40},"total_cost_usd":0.01}"#;
        let event = a.parse_stdout_line(line).expect("must parse");
        match event {
            AgentEvent::StepFinish { part, .. } => {
                assert_eq!(part.reason, "stop");
                assert_eq!(part.session_id, "sess-1");
                let tokens = part.tokens.expect("tokens must be present");
                assert_eq!(tokens.input, 50);
                assert_eq!(tokens.output, 40);
                assert_eq!(tokens.total, 90);
                assert_eq!(part.cost, Some(0.01));
            }
            other => panic!("expected StepFinish, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_stdout_line_result_error() {
        let a = ClaudeAdapter::new();
        let line = r#"{"type":"result","subtype":"error_max_turns","session_id":"sess-1","is_error":true}"#;
        let event = a.parse_stdout_line(line).expect("must parse");
        match event {
            AgentEvent::StepFinish { part, .. } => {
                assert_eq!(part.reason, "error");
            }
            other => panic!("expected StepFinish, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_stdout_line_invalid_json() {
        let a = ClaudeAdapter::new();
        assert!(a.parse_stdout_line("not json").is_none());
    }

    #[test]
    fn test_parse_stdout_line_unknown_type() {
        let a = ClaudeAdapter::new();
        let line = r#"{"type":"some_unknown","session_id":"sess-1"}"#;
        let event = a.parse_stdout_line(line).expect("must parse");
        assert!(matches!(event, AgentEvent::Other));
    }
}
