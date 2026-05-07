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

#[derive(Debug, Clone, Deserialize)]
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

pub fn parse_line(line: &str) -> Option<OpencodeEvent> {
    serde_json::from_str(line).ok()
}
