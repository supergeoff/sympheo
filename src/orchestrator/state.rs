use crate::tracker::model::{Issue, LiveSession, RetryEntry, TokenTotals};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct RunningEntry {
    pub issue: Issue,
    pub session: Option<LiveSession>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub retry_attempt: Option<u32>,
    pub turn_count: u32,
}

#[derive(Debug, Clone)]
pub struct OrchestratorState {
    pub poll_interval_ms: u64,
    pub max_concurrent_agents: usize,
    pub running: HashMap<String, RunningEntry>,
    pub claimed: HashSet<String>,
    pub retry_attempts: HashMap<String, RetryEntry>,
    pub completed: HashSet<String>,
    pub codex_totals: TokenTotals,
    pub codex_rate_limits: Option<serde_json::Value>,
}

impl OrchestratorState {
    pub fn new(poll_interval_ms: u64, max_concurrent_agents: usize) -> Self {
        Self {
            poll_interval_ms,
            max_concurrent_agents,
            running: HashMap::new(),
            claimed: HashSet::new(),
            retry_attempts: HashMap::new(),
            completed: HashSet::new(),
            codex_totals: TokenTotals::default(),
            codex_rate_limits: None,
        }
    }

    pub fn available_slots(&self, _per_state: &HashMap<String, usize>) -> usize {
        let global = self.max_concurrent_agents.saturating_sub(self.running.len());
        // For simplicity, global limit is the primary constraint.
        // Per-state limits would require counting running issues by state.
        global
    }

    pub fn count_running_by_state(&self, state: &str) -> usize {
        let needle = state.to_lowercase();
        self.running
            .values()
            .filter(|e| e.issue.state.to_lowercase() == needle)
            .count()
    }
}
