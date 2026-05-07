use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockerRef {
    pub id: Option<String>,
    pub identifier: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Issue {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<i32>,
    pub state: String,
    pub branch_name: Option<String>,
    pub url: Option<String>,
    pub labels: Vec<String>,
    pub blocked_by: Vec<BlockerRef>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl Issue {
    pub fn is_blocked(&self, terminal_states: &[String]) -> bool {
        self.blocked_by.iter().any(|b| {
            b.state
                .as_ref()
                .map(|s| !terminal_states.contains(&s.to_lowercase()))
                .unwrap_or(false)
        })
    }
}

#[derive(Debug, Clone)]
pub struct WorkflowDefinition {
    pub config: serde_yaml::Mapping,
    pub prompt_template: String,
}

#[derive(Debug, Clone, Default)]
pub struct TokenTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub seconds_running: f64,
}

#[derive(Debug, Clone)]
pub struct LiveSession {
    pub session_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub agent_pid: Option<u32>,
    pub last_event: Option<String>,
    pub last_timestamp: Option<DateTime<Utc>>,
    pub last_message: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub last_reported_input_tokens: u64,
    pub last_reported_output_tokens: u64,
    pub last_reported_total_tokens: u64,
    pub turn_count: u32,
}

#[derive(Debug, Clone)]
pub struct RunAttempt {
    pub issue_id: String,
    pub issue_identifier: String,
    pub attempt: Option<u32>,
    pub workspace_path: std::path::PathBuf,
    pub started_at: DateTime<Utc>,
    pub status: AttemptStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptStatus {
    PreparingWorkspace,
    BuildingPrompt,
    LaunchingAgentProcess,
    InitializingSession,
    StreamingTurn,
    Finishing,
    Succeeded,
    Failed,
    TimedOut,
    Stalled,
    CanceledByReconciliation,
}

#[derive(Debug, Clone)]
pub struct RetryEntry {
    pub issue_id: String,
    pub identifier: String,
    pub attempt: u32,
    pub due_at: std::time::Instant,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceInfo {
    pub path: std::path::PathBuf,
    pub workspace_key: String,
    pub created_now: bool,
}
