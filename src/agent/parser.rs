use serde::Deserialize;

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
    pub total: u64,
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cache: Option<CacheInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CacheInfo {
    pub write: u64,
    pub read: u64,
}

#[derive(Debug, Clone)]
pub struct TurnResult {
    pub session_id: String,
    pub turn_id: String,
    pub success: bool,
    pub text: String,
    pub tokens: Option<TokenInfo>,
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
    RateLimit {
        payload: serde_json::Value,
    },
    #[serde(rename = "token_usage")]
    TokenUsage {
        input: u64,
        output: u64,
        total: u64,
    },
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
            OpencodeEvent::StepStart { session_id, part, .. } => {
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
                assert_eq!(part.time, Some(TextTime { start: 100, end: 200 }));
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
            AgentEvent::TokenUsage { input, output, total } => {
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
}
