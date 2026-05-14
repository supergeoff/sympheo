use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum OpencodeEvent {
    #[serde(rename = "step_start")]
    StepStart {
        timestamp: i64,
        #[serde(rename = "sessionID")]
        session_id: String,
        part: StepStartPart,
    },
    #[serde(rename = "text")]
    Text {
        timestamp: i64,
        #[serde(rename = "sessionID")]
        session_id: String,
        part: TextPart,
    },
    #[serde(rename = "step_finish")]
    StepFinish {
        timestamp: i64,
        #[serde(rename = "sessionID")]
        session_id: String,
        part: StepFinishPart,
    },
    #[serde(rename = "create_pull_request")]
    CreatePullRequest {
        #[serde(rename = "sessionID")]
        session_id: String,
        title: String,
        head: String,
        base: String,
        body: Option<String>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StepStartPart {
    pub id: String,
    #[serde(rename = "messageID")]
    pub message_id: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    #[serde(rename = "type")]
    pub part_type: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TextPart {
    pub id: String,
    #[serde(rename = "messageID")]
    pub message_id: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    #[serde(rename = "type")]
    pub part_type: String,
    pub text: String,
    pub time: Option<TextTime>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct TextTime {
    pub start: i64,
    pub end: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StepFinishPart {
    pub id: String,
    pub reason: String,
    #[serde(rename = "messageID")]
    pub message_id: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    #[serde(rename = "type")]
    pub part_type: String,
    pub tokens: Option<TokenInfo>,
    pub cost: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenInfo {
    #[serde(deserialize_with = "deser_clamped_u64")]
    pub total: u64,
    #[serde(deserialize_with = "deser_clamped_u64")]
    pub input: u64,
    #[serde(deserialize_with = "deser_clamped_u64")]
    pub output: u64,
    #[serde(deserialize_with = "deser_clamped_u64")]
    pub reasoning: u64,
    pub cache: Option<CacheInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CacheInfo {
    #[serde(deserialize_with = "deser_clamped_u64")]
    pub write: u64,
    #[serde(deserialize_with = "deser_clamped_u64")]
    pub read: u64,
}

fn deser_clamped_u64<'de, D>(d: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = i64::deserialize(d)?;
    Ok(v.max(0) as u64)
}

/// ACP-aligned tool kind — what category of operation a tool performs.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    Other,
}

/// Lifecycle status of an in-flight tool call.
/// NOTE: `Cancelled` is intentionally absent — cancellation flows through
/// `PromptResponse`, not through the tool status channel.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// Execution status of a single step inside a `Plan`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Completed,
}

/// A file or code location referenced by a tool call.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Location {
    pub path: String,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
}

/// A single content item returned in a `ToolCallUpdate`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ToolCallContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: Option<String>,
}

/// A single step within a `Plan` event.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PlanStep {
    pub title: String,
    pub status: PlanStepStatus,
}

/// SPEC §10.2.2: outcome of a single agent turn. Replaces the previous
/// `success: bool` so the orchestrator can branch on the specific failure
/// mode (read timeout vs total timeout vs cancelled vs failed) rather than
/// collapsing everything into a boolean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

impl TurnOutcome {
    pub fn is_success(self) -> bool {
        matches!(self, TurnOutcome::Succeeded)
    }
}

#[derive(Debug, Clone)]
pub struct TurnResult {
    pub session_id: String,
    pub turn_id: String,
    /// SPEC §10.2.2.
    pub outcome: TurnOutcome,
    /// SPEC §10.2.2 OPTIONAL `last_message`.
    pub last_message: Option<String>,
    /// SPEC §10.2.2 OPTIONAL `usage`.
    pub usage: Option<TokenInfo>,
    /// SPEC §10.2.2 OPTIONAL normalized error.
    pub error: Option<String>,
}

impl TurnResult {
    pub fn succeeded(&self) -> bool {
        self.outcome.is_success()
    }
}

/// SPEC §10.3 — event payload envelope. Adapters wrap each parsed
/// `AgentEvent` with the active turn's subprocess PID (when known) so the
/// orchestrator can correlate events with workers and operators can trace
/// which subprocess emitted what.
#[derive(Debug, Clone)]
pub struct EmittedEvent {
    pub event: AgentEvent,
    pub agent_pid: Option<u32>,
}

impl EmittedEvent {
    pub fn new(event: AgentEvent) -> Self {
        Self {
            event,
            agent_pid: None,
        }
    }

    pub fn with_pid(event: AgentEvent, pid: Option<u32>) -> Self {
        Self {
            event,
            agent_pid: pid,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum AgentEvent {
    #[serde(rename = "session_started")]
    SessionStarted {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(rename = "threadID")]
        thread_id: String,
    },
    #[serde(rename = "turn_completed")]
    TurnCompleted {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(rename = "turnID")]
        turn_id: String,
        tokens: Option<TokenInfo>,
    },
    #[serde(rename = "turn_failed")]
    TurnFailed {
        #[serde(rename = "sessionID")]
        session_id: String,
        reason: String,
    },
    #[serde(rename = "turn_cancelled")]
    TurnCancelled {
        #[serde(rename = "sessionID")]
        session_id: String,
    },
    #[serde(rename = "turn_input_required")]
    TurnInputRequired {
        #[serde(rename = "sessionID")]
        session_id: String,
    },
    #[serde(rename = "approval_auto_approved")]
    ApprovalAutoApproved {
        #[serde(rename = "sessionID")]
        session_id: String,
        kind: String,
    },
    #[serde(rename = "notification")]
    Notification {
        #[serde(rename = "sessionID")]
        session_id: String,
        message: String,
    },
    #[serde(rename = "rate_limit")]
    RateLimit { payload: serde_json::Value },
    #[serde(rename = "token_usage")]
    TokenUsage { input: u64, output: u64, total: u64 },
    #[serde(rename = "step_start")]
    StepStart {
        timestamp: i64,
        #[serde(rename = "sessionID")]
        session_id: String,
        part: StepStartPart,
    },
    #[serde(rename = "text")]
    Text {
        timestamp: i64,
        #[serde(rename = "sessionID")]
        session_id: String,
        part: TextPart,
    },
    #[serde(rename = "step_finish")]
    StepFinish {
        timestamp: i64,
        #[serde(rename = "sessionID")]
        session_id: String,
        part: StepFinishPart,
    },
    /// ACP `session/update` — tool invocation initiated by the agent.
    #[serde(rename = "tool_call")]
    ToolCall {
        id: String,
        title: String,
        kind: ToolKind,
        raw_input: serde_json::Value,
        #[serde(default)]
        locations: Vec<Location>,
    },
    /// ACP `session/update` — incremental or final update for a tool call.
    /// All fields except `id` are optional to support merge-incremental delivery.
    #[serde(rename = "tool_call_update")]
    ToolCallUpdate {
        id: String,
        status: Option<ToolStatus>,
        #[serde(default)]
        content: Vec<ToolCallContent>,
        raw_output: Option<serde_json::Value>,
    },
    /// ACP `session/update` — file diff produced by a tool call.
    #[serde(rename = "diff")]
    Diff {
        tool_call_id: String,
        path: PathBuf,
        old_text: Option<String>,
        new_text: String,
    },
    /// ACP `session/update` — structured execution plan emitted by the agent.
    #[serde(rename = "plan")]
    Plan {
        #[serde(default)]
        steps: Vec<PlanStep>,
    },
    /// ACP `session/update` — agent reasoning delta (extended thinking).
    #[serde(rename = "thinking")]
    Thinking { delta: String },
    /// Catch-all for ACP types not yet handled (e.g. `available_commands_update`,
    /// `current_mode_update`). Required because ACP types are `#[non_exhaustive]`.
    #[serde(other)]
    Other,
}

pub fn parse_event_line(line: &str) -> Option<AgentEvent> {
    serde_json::from_str(line).ok()
}

pub fn parse_line(line: &str) -> Option<OpencodeEvent> {
    serde_json::from_str(line).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_line_step_start() {
        let json = r#"{"type":"step_start","timestamp":123,"sessionID":"sess-1","part":{"id":"p1","messageID":"msg-1","sessionID":"sess-1","type":"step"}}"#;
        let event = parse_line(json).unwrap();
        match event {
            OpencodeEvent::StepStart {
                session_id, part, ..
            } => {
                assert_eq!(session_id, "sess-1");
                assert_eq!(part.id, "p1");
                assert_eq!(part.message_id, "msg-1");
            }
            _ => panic!("expected StepStart"),
        }
    }

    #[test]
    fn test_parse_line_text() {
        let json = r#"{"type":"text","timestamp":456,"sessionID":"sess-1","part":{"id":"p2","messageID":"msg-2","sessionID":"sess-1","type":"text","text":"hello world","time":{"start":100,"end":200}}}"#;
        let event = parse_line(json).unwrap();
        match event {
            OpencodeEvent::Text { part, .. } => {
                assert_eq!(part.text, "hello world");
                assert_eq!(
                    part.time,
                    Some(TextTime {
                        start: 100,
                        end: 200
                    })
                );
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_parse_line_step_finish() {
        let json = r#"{"type":"step_finish","timestamp":789,"sessionID":"sess-1","part":{"id":"p3","reason":"stop","messageID":"msg-3","sessionID":"sess-1","type":"finish","tokens":{"total":100,"input":50,"output":40,"reasoning":10,"cache":{"write":5,"read":3}},"cost":0.01}}"#;
        let event = parse_line(json).unwrap();
        match event {
            OpencodeEvent::StepFinish { part, .. } => {
                assert_eq!(part.reason, "stop");
                let tokens = part.tokens.unwrap();
                assert_eq!(tokens.total, 100);
                assert_eq!(tokens.input, 50);
                assert_eq!(tokens.output, 40);
                assert_eq!(tokens.reasoning, 10);
                let cache = tokens.cache.unwrap();
                assert_eq!(cache.write, 5);
                assert_eq!(cache.read, 3);
            }
            _ => panic!("expected StepFinish"),
        }
    }

    #[test]
    fn test_parse_line_other() {
        let json = r#"{"type":"unknown_event","timestamp":0}"#;
        let event = parse_line(json).unwrap();
        assert!(matches!(event, OpencodeEvent::Other));
    }

    #[test]
    fn test_parse_line_invalid_json() {
        assert!(parse_line("not json").is_none());
    }

    #[test]
    fn test_parse_line_empty() {
        assert!(parse_line("").is_none());
    }

    #[test]
    fn test_parse_line_step_finish_no_tokens() {
        let json = r#"{"type":"step_finish","timestamp":789,"sessionID":"sess-1","part":{"id":"p3","reason":"tool-calls","messageID":"msg-3","sessionID":"sess-1","type":"finish"}}"#;
        let event = parse_line(json).unwrap();
        match event {
            OpencodeEvent::StepFinish { part, .. } => {
                assert_eq!(part.reason, "tool-calls");
                assert!(part.tokens.is_none());
                assert!(part.cost.is_none());
            }
            _ => panic!("expected StepFinish"),
        }
    }

    #[test]
    fn test_parse_line_text_no_time() {
        let json = r#"{"type":"text","timestamp":456,"sessionID":"sess-1","part":{"id":"p2","messageID":"msg-2","sessionID":"sess-1","type":"text","text":"hello world"}}"#;
        let event = parse_line(json).unwrap();
        match event {
            OpencodeEvent::Text { part, .. } => {
                assert_eq!(part.text, "hello world");
                assert!(part.time.is_none());
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_parse_line_step_finish_tokens_no_cache() {
        let json = r#"{"type":"step_finish","timestamp":789,"sessionID":"sess-1","part":{"id":"p3","reason":"stop","messageID":"msg-3","sessionID":"sess-1","type":"finish","tokens":{"total":100,"input":50,"output":40,"reasoning":10}}}"#;
        let event = parse_line(json).unwrap();
        match event {
            OpencodeEvent::StepFinish { part, .. } => {
                assert_eq!(part.reason, "stop");
                let tokens = part.tokens.unwrap();
                assert_eq!(tokens.total, 100);
                assert!(tokens.cache.is_none());
            }
            _ => panic!("expected StepFinish"),
        }
    }

    #[test]
    fn test_parse_event_line_rate_limit() {
        let json = r#"{"type":"rate_limit","payload":{"limit":100,"remaining":50}}"#;
        let event = parse_event_line(json).unwrap();
        match event {
            AgentEvent::RateLimit { payload } => {
                assert_eq!(payload["limit"], 100);
            }
            _ => panic!("expected RateLimit"),
        }
    }

    #[test]
    fn test_parse_event_line_turn_failed() {
        let json = r#"{"type":"turn_failed","sessionID":"sess-1","reason":"timeout"}"#;
        let event = parse_event_line(json).unwrap();
        match event {
            AgentEvent::TurnFailed { session_id, reason } => {
                assert_eq!(session_id, "sess-1");
                assert_eq!(reason, "timeout");
            }
            _ => panic!("expected TurnFailed"),
        }
    }

    #[test]
    fn test_parse_event_line_token_usage() {
        let json = r#"{"type":"token_usage","input":100,"output":50,"total":150}"#;
        let event = parse_event_line(json).unwrap();
        match event {
            AgentEvent::TokenUsage {
                input,
                output,
                total,
            } => {
                assert_eq!(input, 100);
                assert_eq!(output, 50);
                assert_eq!(total, 150);
            }
            _ => panic!("expected TokenUsage"),
        }
    }

    #[test]
    fn test_parse_event_line_notification() {
        let json = r#"{"type":"notification","sessionID":"sess-1","message":"hello"}"#;
        let event = parse_event_line(json).unwrap();
        match event {
            AgentEvent::Notification { message, .. } => {
                assert_eq!(message, "hello");
            }
            _ => panic!("expected Notification"),
        }
    }

    #[test]
    fn test_parse_event_line_malformed_ignored() {
        assert!(parse_event_line("not json").is_none());
    }

    // --- New ACP variant tests (Lot 2) ---

    #[test]
    fn test_parse_event_line_tool_call() {
        let json = r#"{
            "type": "tool_call",
            "id": "tc-1",
            "title": "Read file",
            "kind": "read",
            "raw_input": {"path": "/tmp/foo.txt"},
            "locations": [{"path": "/tmp/foo.txt", "start_line": 1, "end_line": 10}]
        }"#;
        let event = parse_event_line(json).unwrap();
        match event {
            AgentEvent::ToolCall {
                id,
                title,
                kind,
                raw_input,
                locations,
            } => {
                assert_eq!(id, "tc-1");
                assert_eq!(title, "Read file");
                assert_eq!(kind, ToolKind::Read);
                assert_eq!(raw_input["path"], "/tmp/foo.txt");
                assert_eq!(locations.len(), 1);
                assert_eq!(locations[0].path, "/tmp/foo.txt");
                assert_eq!(locations[0].start_line, Some(1));
                assert_eq!(locations[0].end_line, Some(10));
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn test_parse_event_line_tool_call_update() {
        let json = r#"{
            "type": "tool_call_update",
            "id": "tc-1",
            "status": "completed",
            "content": [{"type": "text", "text": "file contents here"}],
            "raw_output": {"result": "ok"}
        }"#;
        let event = parse_event_line(json).unwrap();
        match event {
            AgentEvent::ToolCallUpdate {
                id,
                status,
                content,
                raw_output,
            } => {
                assert_eq!(id, "tc-1");
                assert_eq!(status, Some(ToolStatus::Completed));
                assert_eq!(content.len(), 1);
                assert_eq!(content[0].content_type, "text");
                assert_eq!(content[0].text.as_deref(), Some("file contents here"));
                assert!(raw_output.is_some());
            }
            _ => panic!("expected ToolCallUpdate"),
        }
    }

    #[test]
    fn test_parse_event_line_tool_call_update_partial() {
        let json = r#"{"type":"tool_call_update","id":"tc-2"}"#;
        let event = parse_event_line(json).unwrap();
        match event {
            AgentEvent::ToolCallUpdate {
                id,
                status,
                content,
                raw_output,
            } => {
                assert_eq!(id, "tc-2");
                assert!(status.is_none());
                assert!(content.is_empty());
                assert!(raw_output.is_none());
            }
            _ => panic!("expected ToolCallUpdate"),
        }
    }

    #[test]
    fn test_parse_event_line_diff() {
        let json = r#"{
            "type": "diff",
            "tool_call_id": "tc-1",
            "path": "/tmp/foo.rs",
            "old_text": "fn old() {}",
            "new_text": "fn new() {}"
        }"#;
        let event = parse_event_line(json).unwrap();
        match event {
            AgentEvent::Diff {
                tool_call_id,
                path,
                old_text,
                new_text,
            } => {
                assert_eq!(tool_call_id, "tc-1");
                assert_eq!(path, PathBuf::from("/tmp/foo.rs"));
                assert_eq!(old_text.as_deref(), Some("fn old() {}"));
                assert_eq!(new_text, "fn new() {}");
            }
            _ => panic!("expected Diff"),
        }
    }

    #[test]
    fn test_parse_event_line_diff_no_old_text() {
        let json = r#"{
            "type": "diff",
            "tool_call_id": "tc-3",
            "path": "/tmp/new.rs",
            "new_text": "fn created() {}"
        }"#;
        let event = parse_event_line(json).unwrap();
        match event {
            AgentEvent::Diff {
                old_text, new_text, ..
            } => {
                assert!(old_text.is_none());
                assert_eq!(new_text, "fn created() {}");
            }
            _ => panic!("expected Diff"),
        }
    }

    #[test]
    fn test_parse_event_line_plan() {
        let json = r#"{
            "type": "plan",
            "steps": [
                {"title": "Step 1", "status": "pending"},
                {"title": "Step 2", "status": "in_progress"},
                {"title": "Step 3", "status": "completed"}
            ]
        }"#;
        let event = parse_event_line(json).unwrap();
        match event {
            AgentEvent::Plan { steps } => {
                assert_eq!(steps.len(), 3);
                assert_eq!(steps[0].title, "Step 1");
                assert_eq!(steps[0].status, PlanStepStatus::Pending);
                assert_eq!(steps[1].status, PlanStepStatus::InProgress);
                assert_eq!(steps[2].status, PlanStepStatus::Completed);
            }
            _ => panic!("expected Plan"),
        }
    }

    #[test]
    fn test_parse_event_line_thinking() {
        let json = r#"{"type":"thinking","delta":"I should read the file first"}"#;
        let event = parse_event_line(json).unwrap();
        match event {
            AgentEvent::Thinking { delta } => {
                assert_eq!(delta, "I should read the file first");
            }
            _ => panic!("expected Thinking"),
        }
    }

    #[test]
    fn test_tool_status_no_cancelled() {
        let result: Result<ToolStatus, _> = serde_json::from_str(r#""cancelled""#);
        assert!(
            result.is_err(),
            "ToolStatus must not have a Cancelled variant"
        );
    }

    #[test]
    fn test_parse_event_line_available_commands_update_is_other() {
        let json = r#"{"type":"available_commands_update","commands":["help","quit"]}"#;
        let event = parse_event_line(json).unwrap();
        assert!(
            matches!(event, AgentEvent::Other),
            "available_commands_update should map to Other"
        );
    }

    #[test]
    fn test_parse_event_line_current_mode_update_is_other() {
        let json = r#"{"type":"current_mode_update","mode":"build"}"#;
        let event = parse_event_line(json).unwrap();
        assert!(
            matches!(event, AgentEvent::Other),
            "current_mode_update should map to Other"
        );
    }
}
