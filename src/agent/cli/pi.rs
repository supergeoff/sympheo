//! SPEC §10 pi.dev CLI adapter (`packages/coding-agent` from
//! `earendil-works/pi-mono`, version-tested against 0.74+).
//!
//! Pi has no subcommand: non-interactive mode is selected by `--mode json`,
//! which emits one JSON object per line on stdout. The prompt is passed as a
//! positional argument; the working directory is inherited from the spawning
//! process (pi reads `process.cwd()` and exposes no `--cwd`/`--dir` flag), so
//! `LocalBackend` MUST `cmd.current_dir(workspace_path)` before spawning (it
//! already does — see SPEC §9.5 Inv 1).
//!
//! Stdout protocol (subset Sympheo reacts to):
//!   {"type":"session","version":3,"id":"<uuid>","timestamp":"...","cwd":"..."}
//!   {"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"..."}, ...}
//!   {"type":"turn_end", "message": {..., "usage": {...}, "stopReason": "stop"}, ...}
//!   {"type":"agent_end", "messages": [...]}
//!
//! Pi has no native permission-mode flag analogous to claude's
//! `--permission-mode`. Setting `cli.options.permission` is therefore a no-op
//! for this adapter (logged as a warning so operators can spot the gap).

use crate::agent::cli::{
    CliAdapter, CliOptions, append_additional_args, append_flag, is_uuid, shell_escape,
};
use crate::agent::parser::{
    AgentEvent, StepFinishPart, StepStartPart, TextPart, TextTime, TokenInfo,
};
use async_trait::async_trait;
use std::path::Path;

/// Tested pi.dev CLI version range (advisory; not enforced at runtime).
pub const SUPPORTED_PI_VERSION_RANGE: &str = ">=0.74";

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

    fn build_command_string(
        &self,
        cli_command: &str,
        prompt_path: &Path,
        _workspace_path: &Path,
        session_id: Option<&str>,
        cli_options: &CliOptions,
    ) -> String {
        // `--mode json` is the JSONL-output mode (vs `text` and `rpc`). Pi
        // exposes no `--cwd`/`--dir` flag — the workspace is inherited from
        // the spawning process's current_dir, which `LocalBackend` already
        // sets to `workspace_path` per SPEC §9.5 Inv 1.
        let mut cmd = format!(
            r#"PROMPT=$(cat "{}"); {} --mode json "$PROMPT""#,
            shell_escape(&prompt_path.to_string_lossy()),
            cli_command,
        );
        if let Some(model) = &cli_options.model {
            append_flag(&mut cmd, "--model", model);
        }
        if let Some(permission) = cli_options.permission {
            tracing::warn!(
                adapter = "pidev",
                permission = permission.as_str(),
                "cli.options.permission is set but pi has no native permission-mode flag; ignoring"
            );
        }
        // `pi --session <id>` resolves the id against on-disk sessions; pi
        // refuses unknown ids with "No session found matching '<id>'". Pi
        // mints the session UUID itself on first run (visible in the first
        // JSONL line's `id` field), so we only forward `--session` when the
        // caller's value already looks like a UUID. First turn (no
        // continuation) always omits the flag.
        if let Some(sid) = session_id.filter(|s| is_uuid(s)) {
            append_flag(&mut cmd, "--session", sid);
        }
        append_additional_args(&mut cmd, &cli_options.additional_args);
        cmd
    }

    fn parse_stdout_line(&self, line: &str) -> Option<AgentEvent> {
        // Pi emits one JSON object per line in `--mode json`. We map the
        // shapes that drive the orchestrator's run loop onto the shared
        // `AgentEvent` variants used by every adapter.
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        let kind = v.get("type")?.as_str()?;
        let session_id = v
            .get("id")
            .and_then(|s| s.as_str())
            .or_else(|| v.get("sessionId").and_then(|s| s.as_str()))
            .unwrap_or("pi-session")
            .to_string();
        match kind {
            "session" => {
                // First line of every run; carries the session UUID pi minted.
                Some(AgentEvent::StepStart {
                    timestamp: 0,
                    session_id: session_id.clone(),
                    part: StepStartPart {
                        id: format!("{session_id}-init"),
                        message_id: format!("{session_id}-init"),
                        session_id,
                        part_type: "session_header".into(),
                    },
                })
            }
            "message_update" => {
                // `assistantMessageEvent` is the streaming delta envelope. We
                // only forward text deltas as `Text`; tool-use deltas land in
                // `Other` so the run-loop keeps polling without surfacing a
                // half-rendered tool invocation.
                let ev = v.get("assistantMessageEvent")?;
                let ev_type = ev.get("type")?.as_str()?;
                if ev_type != "text_delta" {
                    return Some(AgentEvent::Other);
                }
                let delta = ev.get("delta").and_then(|d| d.as_str()).unwrap_or("");
                if delta.is_empty() {
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
                        text: delta.to_string(),
                        time: Some(TextTime { start: 0, end: 0 }),
                    },
                })
            }
            "turn_end" | "agent_end" => {
                // Both events carry the terminating assistant message. Read
                // `stopReason` to classify success vs. error, and sum
                // `usage` for the token tally. `turn_end` carries
                // `{message, toolResults}`; `agent_end` carries
                // `{messages: AgentMessage[]}` — we read the last assistant
                // message in either case.
                let assistant_msg = v.get("message").or_else(|| {
                    v.get("messages")
                        .and_then(|arr| arr.as_array())
                        .and_then(|arr| {
                            arr.iter().rev().find(|m| {
                                m.get("role").and_then(|r| r.as_str()) == Some("assistant")
                            })
                        })
                });
                let stop_reason = assistant_msg
                    .and_then(|m| m.get("stopReason"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("stop");
                let reason = match stop_reason {
                    "error" | "aborted" => "error".to_string(),
                    _ => "stop".to_string(),
                };
                let tokens = assistant_msg.and_then(|m| m.get("usage")).map(|u| {
                    let input = u.get("input").and_then(|n| n.as_u64()).unwrap_or(0);
                    let output = u.get("output").and_then(|n| n.as_u64()).unwrap_or(0);
                    let total = u
                        .get("totalTokens")
                        .and_then(|n| n.as_u64())
                        .unwrap_or(input + output);
                    TokenInfo {
                        total,
                        input,
                        output,
                        reasoning: 0,
                        cache: None,
                    }
                });
                let cost = assistant_msg
                    .and_then(|m| m.get("usage"))
                    .and_then(|u| u.get("cost"))
                    .and_then(|c| c.get("total"))
                    .and_then(|n| n.as_f64());
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
                        cost,
                    },
                })
            }
            _ => Some(AgentEvent::Other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::cli::Permission;
    use crate::error::SympheoError;

    #[test]
    fn test_kind_and_names() {
        let a = PiAdapter::new();
        assert_eq!(a.kind(), "pidev");
        assert_eq!(a.binary_names(), &["pi"]);
    }

    #[test]
    fn test_validate_pi_run() {
        assert!(PiAdapter::new().validate("pi").is_ok());
        assert!(PiAdapter::new().validate("pi --mode json").is_ok());
        assert!(PiAdapter::new().validate("/usr/local/bin/pi").is_ok());
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

    #[test]
    fn test_supported_version_range_is_set() {
        assert!(!SUPPORTED_PI_VERSION_RANGE.is_empty());
    }

    #[test]
    fn test_build_command_string_first_turn() {
        let a = PiAdapter::new();
        let prompt = Path::new("/ws/.sympheo_prompt.txt");
        let ws = Path::new("/ws");
        let cmd = a.build_command_string("pi", prompt, ws, None, &CliOptions::default());
        assert!(cmd.contains("pi --mode json"));
        assert!(cmd.contains("\"$PROMPT\""));
        assert!(
            !cmd.contains("--session"),
            "first turn must not include --session"
        );
        assert!(!cmd.contains("--dir"), "pi exposes no --dir flag");
    }

    #[test]
    fn test_build_command_string_splices_model_and_args() {
        let a = PiAdapter::new();
        let prompt = Path::new("/ws/.sympheo_prompt.txt");
        let ws = Path::new("/ws");
        let opts = CliOptions {
            model: Some("sonnet:high".into()),
            permission: None,
            additional_args: vec!["--thinking".into(), "high".into()],
        };
        let cmd = a.build_command_string("pi", prompt, ws, None, &opts);
        assert!(cmd.contains("--model sonnet:high"));
        assert!(cmd.contains("--thinking"));
        assert!(cmd.contains(" high"));
    }

    #[test]
    fn test_build_command_string_with_uuid_session() {
        let a = PiAdapter::new();
        let prompt = Path::new("/ws/.sympheo_prompt.txt");
        let ws = Path::new("/ws");
        let uuid = "33595f82-f956-4338-854d-f6332a296842";
        let cmd = a.build_command_string("pi", prompt, ws, Some(uuid), &CliOptions::default());
        assert!(cmd.contains(&format!("--session {uuid}")));
    }

    #[test]
    fn test_build_command_string_skips_session_for_non_uuid() {
        let a = PiAdapter::new();
        let prompt = Path::new("/ws/.sympheo_prompt.txt");
        let ws = Path::new("/ws");
        let cmd = a.build_command_string(
            "pi",
            prompt,
            ws,
            Some("pidev-123-456"),
            &CliOptions::default(),
        );
        assert!(!cmd.contains("--session"));
    }

    #[test]
    fn test_build_command_string_permission_is_noop() {
        let a = PiAdapter::new();
        let prompt = Path::new("/ws/.sympheo_prompt.txt");
        let ws = Path::new("/ws");
        let opts = CliOptions {
            permission: Some(Permission::Plan),
            ..Default::default()
        };
        let cmd = a.build_command_string("pi", prompt, ws, None, &opts);
        assert!(!cmd.contains("--permission"));
        assert!(!cmd.contains("plan"));
    }

    #[test]
    fn test_parse_stdout_line_session_header() {
        let a = PiAdapter::new();
        let line = r#"{"type":"session","version":3,"id":"33595f82-f956-4338-854d-f6332a296842","timestamp":"2026-05-11T12:00:00Z","cwd":"/ws"}"#;
        let event = a.parse_stdout_line(line).expect("must parse");
        match event {
            AgentEvent::StepStart {
                session_id, part, ..
            } => {
                assert_eq!(session_id, "33595f82-f956-4338-854d-f6332a296842");
                assert_eq!(part.part_type, "session_header");
            }
            other => panic!("expected StepStart, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_stdout_line_text_delta() {
        let a = PiAdapter::new();
        let line = r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"hello "}}"#;
        let event = a.parse_stdout_line(line).expect("must parse");
        match event {
            AgentEvent::Text { part, .. } => assert_eq!(part.text, "hello "),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_stdout_line_non_text_delta_is_other() {
        let a = PiAdapter::new();
        let line =
            r#"{"type":"message_update","assistantMessageEvent":{"type":"tool_call_delta"}}"#;
        let event = a.parse_stdout_line(line).expect("must parse");
        assert!(matches!(event, AgentEvent::Other));
    }

    #[test]
    fn test_parse_stdout_line_turn_end_success() {
        let a = PiAdapter::new();
        let line = r#"{"type":"turn_end","message":{"role":"assistant","stopReason":"stop","usage":{"input":12,"output":34,"totalTokens":46,"cost":{"total":0.002}}}}"#;
        let event = a.parse_stdout_line(line).expect("must parse");
        match event {
            AgentEvent::StepFinish { part, .. } => {
                assert_eq!(part.reason, "stop");
                let tokens = part.tokens.expect("tokens must be present");
                assert_eq!(tokens.input, 12);
                assert_eq!(tokens.output, 34);
                assert_eq!(tokens.total, 46);
                assert_eq!(part.cost, Some(0.002));
            }
            other => panic!("expected StepFinish, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_stdout_line_agent_end_picks_last_assistant() {
        let a = PiAdapter::new();
        let line = r#"{"type":"agent_end","messages":[{"role":"user"},{"role":"assistant","stopReason":"error","usage":{"input":1,"output":2}}]}"#;
        let event = a.parse_stdout_line(line).expect("must parse");
        match event {
            AgentEvent::StepFinish { part, .. } => assert_eq!(part.reason, "error"),
            other => panic!("expected StepFinish, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_stdout_line_invalid_json_returns_none() {
        let a = PiAdapter::new();
        assert!(a.parse_stdout_line("not json").is_none());
    }

    #[test]
    fn test_parse_stdout_line_unknown_type_is_other() {
        let a = PiAdapter::new();
        let line = r#"{"type":"queue_update"}"#;
        let event = a.parse_stdout_line(line).expect("must parse");
        assert!(matches!(event, AgentEvent::Other));
    }
}
