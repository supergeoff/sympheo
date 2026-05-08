use crate::skills::Skill;
use crate::tracker::model::{Issue, LiveSession, RetryEntry, TokenTotals};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct RunningEntry {
    pub issue: Issue,
    pub session: Option<LiveSession>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub retry_attempt: Option<u32>,
    pub turn_count: u32,
    pub cancelled: Arc<AtomicBool>,
}

impl RunningEntry {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
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
    pub skills: HashMap<String, Skill>,
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
            skills: HashMap::new(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_new() {
        let state = OrchestratorState::new(5000, 5);
        assert_eq!(state.poll_interval_ms, 5000);
        assert_eq!(state.max_concurrent_agents, 5);
        assert!(state.running.is_empty());
        assert!(state.claimed.is_empty());
        assert!(state.retry_attempts.is_empty());
        assert!(state.completed.is_empty());
        assert_eq!(state.codex_totals.input_tokens, 0);
        assert!(state.codex_rate_limits.is_none());
    }

    #[test]
    fn test_available_slots() {
        let mut state = OrchestratorState::new(10000, 3);
        assert_eq!(state.available_slots(&HashMap::new()), 3);

        state.running.insert(
            "1".into(),
            RunningEntry {
                issue: Issue {
                    id: "1".into(),
                    identifier: "TEST-1".into(),
                    title: "a".into(),
                    description: None,
                    priority: None,
                    state: "todo".into(),
                    branch_name: None,
                    url: None,
                    labels: vec![],
                    blocked_by: vec![],
                    created_at: None,
                    updated_at: None,
                },
                session: None,
                started_at: chrono::Utc::now(),
                retry_attempt: None,
                turn_count: 0,
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        );
        assert_eq!(state.available_slots(&HashMap::new()), 2);
    }

    #[test]
    fn test_available_slots_saturating() {
        let mut state = OrchestratorState::new(10000, 1);
        state.running.insert(
            "1".into(),
            RunningEntry {
                issue: Issue {
                    id: "1".into(),
                    identifier: "TEST-1".into(),
                    title: "a".into(),
                    description: None,
                    priority: None,
                    state: "todo".into(),
                    branch_name: None,
                    url: None,
                    labels: vec![],
                    blocked_by: vec![],
                    created_at: None,
                    updated_at: None,
                },
                session: None,
                started_at: chrono::Utc::now(),
                retry_attempt: None,
                turn_count: 0,
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        );
        assert_eq!(state.available_slots(&HashMap::new()), 0);
    }

    #[test]
    fn test_count_running_by_state() {
        let mut state = OrchestratorState::new(10000, 10);
        for (id, st) in [("1", "todo"), ("2", "in progress"), ("3", "todo")] {
            state.running.insert(
                id.into(),
                RunningEntry {
                    issue: Issue {
                        id: id.into(),
                        identifier: format!("TEST-{id}"),
                        title: "a".into(),
                        description: None,
                        priority: None,
                        state: st.into(),
                        branch_name: None,
                        url: None,
                        labels: vec![],
                        blocked_by: vec![],
                        created_at: None,
                        updated_at: None,
                    },
                    session: None,
                    started_at: chrono::Utc::now(),
                    retry_attempt: None,
                    turn_count: 0,
                    cancelled: Arc::new(AtomicBool::new(false)),
                },
            );
        }
        assert_eq!(state.count_running_by_state("todo"), 2);
        assert_eq!(state.count_running_by_state("in progress"), 1);
        assert_eq!(state.count_running_by_state("closed"), 0);
    }

    #[test]
    fn test_running_entry_is_cancelled() {
        let entry = RunningEntry {
            issue: Issue {
                id: "1".into(),
                identifier: "TEST-1".into(),
                title: "a".into(),
                description: None,
                priority: None,
                state: "todo".into(),
                branch_name: None,
                url: None,
                labels: vec![],
                blocked_by: vec![],
                created_at: None,
                updated_at: None,
            },
            session: None,
            started_at: chrono::Utc::now(),
            retry_attempt: None,
            turn_count: 0,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        assert!(!entry.is_cancelled());
        entry.cancelled.store(true, Ordering::Relaxed);
        assert!(entry.is_cancelled());
    }
}
