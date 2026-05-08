use crate::agent::runner::AgentRunner;
use crate::config::typed::ServiceConfig;
use crate::error::SympheoError;
use crate::orchestrator::retry::schedule_retry;
use crate::orchestrator::state::{OrchestratorState, RunningEntry};
use crate::tracker::model::{Issue, LiveSession};
use crate::tracker::IssueTracker;
use crate::workspace::manager::WorkspaceManager;
use chrono::Utc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

pub struct Orchestrator {
    pub state: Arc<RwLock<OrchestratorState>>,
    pub config: Arc<RwLock<ServiceConfig>>,
    tracker: Arc<dyn IssueTracker>,
    runner: Arc<AgentRunner>,
    workspace_manager: Arc<WorkspaceManager>,
}

impl Orchestrator {
    pub fn new(
        config: ServiceConfig,
        tracker: Arc<dyn IssueTracker>,
    ) -> Result<Self, SympheoError> {
        let state = OrchestratorState::new(
            config.poll_interval_ms(),
            config.max_concurrent_agents(),
        );
        let workspace_manager = WorkspaceManager::new(&config)?;
        let runner = AgentRunner::new(&config)?;
        Ok(Self {
            state: Arc::new(RwLock::new(state)),
            config: Arc::new(RwLock::new(config)),
            tracker,
            runner: Arc::new(runner),
            workspace_manager: Arc::new(workspace_manager),
        })
    }

    pub async fn reload_config(&self, config: ServiceConfig) {
        let mut state = self.state.write().await;
        state.poll_interval_ms = config.poll_interval_ms();
        state.max_concurrent_agents = config.max_concurrent_agents();
        *self.config.write().await = config;
    }

    pub async fn tick(&self) {
        info!("orchestrator tick start");

        // Part A: Reconcile
        if let Err(e) = self.reconcile().await {
            warn!(error = %e, "reconciliation failed");
        }

        // Preflight validation
        let config = self.config.read().await.clone();
        if let Err(e) = config.validate_for_dispatch() {
            warn!(error = %e, "dispatch validation failed, skipping dispatch");
            return;
        }

        // Fetch candidates
        let candidates = match self.tracker.fetch_candidate_issues().await {
            Ok(issues) => issues,
            Err(e) => {
                warn!(error = %e, "candidate fetch failed");
                return;
            }
        };

        let active_states = config.active_states();
        let terminal_states = config.terminal_states();
        let max_turns = config.max_turns();
        let per_state_limits = config.max_concurrent_agents_by_state();

        let mut eligible: Vec<Issue> = candidates
            .into_iter()
            .filter(|i| {
                let state_lc = i.state.to_lowercase();
                active_states.contains(&state_lc) && !terminal_states.contains(&state_lc)
            })
            .filter(|i| {
                let state_lc = i.state.to_lowercase();
                if state_lc == "todo" {
                    !i.is_blocked(&terminal_states)
                } else {
                    true
                }
            })
            .collect();

        // Sort by priority asc, created_at asc, identifier asc
        eligible.sort_by(|a, b| {
            a.priority
                .unwrap_or(i32::MAX)
                .cmp(&b.priority.unwrap_or(i32::MAX))
                .then_with(|| a.created_at.cmp(&b.created_at))
                .then_with(|| a.identifier.cmp(&b.identifier))
        });

        let mut state = self.state.write().await;
        for issue in eligible {
            if state.available_slots(&per_state_limits) == 0 {
                break;
            }
            if state.running.contains_key(&issue.id) || state.claimed.contains(&issue.id) {
                continue;
            }

            let state_limit = per_state_limits
                .get(&issue.state.to_lowercase())
                .copied()
                .unwrap_or(state.max_concurrent_agents);
            if state.count_running_by_state(&issue.state) >= state_limit {
                continue;
            }

            // Dispatch
            let issue_id = issue.id.clone();
            let cancelled = Arc::new(AtomicBool::new(false));
            state.claimed.insert(issue_id.clone());
            state.running.insert(
                issue_id.clone(),
                RunningEntry {
                    issue: issue.clone(),
                    session: None,
                    started_at: Utc::now(),
                    retry_attempt: None,
                    turn_count: 0,
                    cancelled: cancelled.clone(),
                },
            );
            drop(state); // release lock before spawning

            self.spawn_worker(issue, None, max_turns, cancelled);

            state = self.state.write().await;
        }

        info!("orchestrator tick end");
    }

    async fn reconcile(&self) -> Result<(), SympheoError> {
        let running_ids: Vec<String> = {
            let state = self.state.read().await;
            state.running.keys().cloned().collect()
        };

        if running_ids.is_empty() {
            return Ok(());
        }

        let config = self.config.read().await.clone();
        let stall_timeout_ms = config.codex_stall_timeout_ms();

        // Stall detection
        let now = Utc::now();
        let mut to_kill = vec![];
        {
            let state = self.state.read().await;
            for (id, entry) in &state.running {
                if stall_timeout_ms <= 0 {
                    continue;
                }
                let elapsed = if let Some(ref sess) = entry.session {
                    if let Some(ts) = sess.last_timestamp {
                        (now - ts).num_milliseconds() as u64
                    } else {
                        (now - entry.started_at).num_milliseconds() as u64
                    }
                } else {
                    (now - entry.started_at).num_milliseconds() as u64
                };
                if elapsed > stall_timeout_ms as u64 {
                    to_kill.push(id.clone());
                }
            }
        }

        for id in to_kill {
            warn!(issue_id = %id, "stall detected, terminating");
            self.handle_worker_exit(&id, false, Some("stalled".into())).await;
        }

        // Tracker state refresh
        let refreshed = self
            .tracker
            .fetch_issue_states_by_ids(&running_ids)
            .await?;

        let active_states = config.active_states();
        let terminal_states = config.terminal_states();

        let mut state = self.state.write().await;
        for issue in refreshed {
            let state_lc = issue.state.to_lowercase();
            if terminal_states.contains(&state_lc) {
                if let Some(entry) = state.running.get(&issue.id) {
                    entry.cancelled.store(true, Ordering::Relaxed);
                }
                if let Some(entry) = state.running.remove(&issue.id) {
                    state.claimed.remove(&issue.id);
                    let identifier = entry.issue.identifier.clone();
                    drop(state);
                    let ws_path = self.workspace_manager.workspace_path(&identifier);
                    if let Err(e) = self.runner.cleanup_workspace(&ws_path).await {
                        warn!(error = %e, "daytona cleanup failed during reconcile");
                    }
                    self.workspace_manager
                        .remove_workspace(&identifier, config.hook_script("before_remove").as_deref())
                        .await;
                    state = self.state.write().await;
                }
            } else if active_states.contains(&state_lc) {
                if let Some(entry) = state.running.get_mut(&issue.id) {
                    entry.issue = issue;
                }
            } else {
                if let Some(entry) = state.running.get(&issue.id) {
                    entry.cancelled.store(true, Ordering::Relaxed);
                }
                if state.running.remove(&issue.id).is_some() {
                    state.claimed.remove(&issue.id);
                }
            }
        }

        Ok(())
    }

    fn spawn_worker(&self, issue: Issue, attempt: Option<u32>, max_turns: u32, cancelled: Arc<AtomicBool>) {
        let state = self.state.clone();
        let config = self.config.clone();
        let runner = self.runner.clone();
        let tracker = self.tracker.clone();
        let workspace_manager = self.workspace_manager.clone();

        tokio::spawn(async move {
            let cfg = config.read().await.clone();
            let result = run_worker(
                issue.clone(),
                attempt,
                max_turns,
                &cfg,
                runner.as_ref(),
                tracker.as_ref(),
                workspace_manager.as_ref(),
                state.clone(),
                cancelled,
            )
            .await;

            let mut st = state.write().await;
            // Accumulate runtime for the finished session
            if let Some(entry) = st.running.get(&issue.id) {
                let elapsed = (Utc::now() - entry.started_at).num_seconds() as f64;
                st.codex_totals.seconds_running += elapsed;
            }
            match result {
                Ok(()) => {
                    info!(issue_id = %issue.id, "worker exited normally");
                    if let Some(entry) = st.running.remove(&issue.id) {
                        st.completed.insert(issue.id.clone());
                        let retry = schedule_retry(
                            issue.id.clone(),
                            entry.issue.identifier.clone(),
                            1,
                            None,
                            &cfg,
                            true,
                        );
                        st.retry_attempts.insert(issue.id.clone(), retry);
                    }
                }
                Err(e) => {
                    error!(issue_id = %issue.id, error = %e, "worker failed");
                    if let Some(entry) = st.running.remove(&issue.id) {
                        let next_attempt = attempt.unwrap_or(0) + 1;
                        let retry = schedule_retry(
                            issue.id.clone(),
                            entry.issue.identifier.clone(),
                            next_attempt,
                            Some(e.to_string()),
                            &cfg,
                            false,
                        );
                        st.retry_attempts.insert(issue.id.clone(), retry);
                    }
                }
            }
        });
    }

    pub async fn handle_worker_exit(
        &self,
        issue_id: &str,
        normal: bool,
        error: Option<String>,
    ) {
        let mut state = self.state.write().await;
        let cfg = self.config.read().await.clone();
        if let Some(entry) = state.running.remove(issue_id) {
            let elapsed = (Utc::now() - entry.started_at).num_seconds() as f64;
            state.codex_totals.seconds_running += elapsed;
            if normal {
                state.completed.insert(issue_id.to_string());
                let retry = schedule_retry(
                    issue_id.to_string(),
                    entry.issue.identifier.clone(),
                    1,
                    None,
                    &cfg,
                    true,
                );
                state.retry_attempts.insert(issue_id.to_string(), retry);
            } else {
                let next_attempt = entry.retry_attempt.unwrap_or(0) + 1;
                let retry = schedule_retry(
                    issue_id.to_string(),
                    entry.issue.identifier.clone(),
                    next_attempt,
                    error,
                    &cfg,
                    false,
                );
                state.retry_attempts.insert(issue_id.to_string(), retry);
            }
        }
    }

    pub async fn process_retries(&self) {
        let now = Instant::now();
        let due: Vec<(String, crate::tracker::model::RetryEntry)> = {
            let state = self.state.read().await;
            state
                .retry_attempts
                .iter()
                .filter(|(_, entry)| entry.due_at <= now)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        };

        for (issue_id, retry) in due {
            let mut state = self.state.write().await;
            state.retry_attempts.remove(&issue_id);
            drop(state);

            let candidates = match self.tracker.fetch_candidate_issues().await {
                Ok(c) => c,
                Err(e) => {
                    warn!(issue_id = %issue_id, error = %e, "retry candidate fetch failed, requeuing");
                    let config = self.config.read().await.clone();
                    let new_retry = schedule_retry(
                        issue_id.clone(),
                        retry.identifier.clone(),
                        retry.attempt + 1,
                        Some("retry poll failed".into()),
                        &config,
                        false,
                    );
                    let mut st = self.state.write().await;
                    st.retry_attempts.insert(issue_id, new_retry);
                    continue;
                }
            };

            let config = self.config.read().await.clone();
            if let Some(issue) = candidates.into_iter().find(|i| i.id == issue_id) {
                let active = config.active_states();
                let terminal = config.terminal_states();
                let state_lc = issue.state.to_lowercase();
                if terminal.contains(&state_lc) || !active.contains(&state_lc) {
                    let mut st = self.state.write().await;
                    st.claimed.remove(&issue_id);
                    continue;
                }

                let mut st = self.state.write().await;
                if st.available_slots(&config.max_concurrent_agents_by_state()) == 0 {
                    let new_retry = schedule_retry(
                        issue_id.clone(),
                        issue.identifier.clone(),
                        retry.attempt,
                        Some("no available orchestrator slots".into()),
                        &config,
                        false,
                    );
                    st.retry_attempts.insert(issue_id, new_retry);
                    continue;
                }
                let cancelled = Arc::new(AtomicBool::new(false));
                st.claimed.insert(issue_id.clone());
                st.running.insert(
                    issue_id.clone(),
                    RunningEntry {
                        issue: issue.clone(),
                        session: None,
                        started_at: Utc::now(),
                        retry_attempt: Some(retry.attempt),
                        turn_count: 0,
                        cancelled: cancelled.clone(),
                    },
                );
                drop(st);
                self.spawn_worker(issue, Some(retry.attempt), config.max_turns(), cancelled);
            } else {
                let mut st = self.state.write().await;
                st.claimed.remove(&issue_id);
            }
        }
    }
}

use std::time::Instant;

#[allow(clippy::too_many_arguments)]
async fn run_worker(
    issue: Issue,
    attempt: Option<u32>,
    max_turns: u32,
    config: &ServiceConfig,
    runner: &AgentRunner,
    tracker: &dyn IssueTracker,
    workspace_manager: &WorkspaceManager,
    state: Arc<RwLock<OrchestratorState>>,
    cancelled: Arc<AtomicBool>,
) -> Result<(), SympheoError> {
    let workspace = workspace_manager
        .create_or_reuse(
            &issue.identifier,
            config.hook_script("after_create").as_deref(),
        )
        .await?;

    if let Some(script) = config.hook_script("before_run") {
        workspace_manager
            .run_hook("before_run", &script, &workspace.path)
            .await?;
    }

    let mut current_session: Option<String> = None;
    let mut turn_number = 1;

    loop {
        if cancelled.load(Ordering::Relaxed) {
            info!(issue_id = %issue.id, "worker cancelled by orchestrator, stopping");
            break;
        }

        let prompt = if turn_number == 1 {
            build_prompt(config, &issue, attempt)?
        } else {
            "Continue working on the current task. Review the conversation history and proceed with the next step.".to_string()
        };

        let turn_result = runner
            .run_turn(&issue, &prompt, current_session.as_deref(), &workspace.path)
            .await?;

        // Update orchestrator state with session metadata and tokens
        {
            let mut st = state.write().await;
            if let Some(entry) = st.running.get_mut(&issue.id) {
                entry.turn_count += 1;
                entry.session = Some(LiveSession {
                    session_id: format!("{}-{}", turn_result.session_id, turn_result.turn_id),
                    thread_id: turn_result.session_id.clone(),
                    turn_id: turn_result.turn_id.clone(),
                    agent_pid: None,
                    last_event: Some("turn_completed".into()),
                    last_timestamp: Some(Utc::now()),
                    last_message: Some(turn_result.text.clone()),
                    input_tokens: turn_result.tokens.as_ref().map(|t| t.input).unwrap_or(0),
                    output_tokens: turn_result.tokens.as_ref().map(|t| t.output).unwrap_or(0),
                    total_tokens: turn_result.tokens.as_ref().map(|t| t.total).unwrap_or(0),
                    last_reported_input_tokens: turn_result.tokens.as_ref().map(|t| t.input).unwrap_or(0),
                    last_reported_output_tokens: turn_result.tokens.as_ref().map(|t| t.output).unwrap_or(0),
                    last_reported_total_tokens: turn_result.tokens.as_ref().map(|t| t.total).unwrap_or(0),
                    turn_count: entry.turn_count,
                });
                if let Some(ref tokens) = turn_result.tokens {
                    st.codex_totals.input_tokens += tokens.input;
                    st.codex_totals.output_tokens += tokens.output;
                    st.codex_totals.total_tokens += tokens.total;
                }
            }
        }

        if !turn_result.success {
            return Err(SympheoError::AgentRunnerError(
                "turn reported failure".into(),
            ));
        }

        current_session = Some(turn_result.session_id);

        // Refresh issue state
        let refreshed = tracker
            .fetch_issue_states_by_ids(std::slice::from_ref(&issue.id))
            .await?;
        let active_states = config.active_states();
        let terminal_states = config.terminal_states();

        if let Some(refreshed_issue) = refreshed.into_iter().next() {
            let state_lc = refreshed_issue.state.to_lowercase();
            if terminal_states.contains(&state_lc) || !active_states.contains(&state_lc) {
                break;
            }
        }

        if turn_number >= max_turns {
            break;
        }
        turn_number += 1;
    }

    if let Some(script) = config.hook_script("after_run") {
        if let Err(e) = workspace_manager.run_hook("after_run", &script, &workspace.path).await {
            warn!(error = %e, "after_run hook failed");
        }
    }

    // Cleanup Daytona sandbox if issue is now terminal
    let refreshed = tracker
        .fetch_issue_states_by_ids(std::slice::from_ref(&issue.id))
        .await?;
    let active_states = config.active_states();
    let terminal_states = config.terminal_states();
    if let Some(refreshed_issue) = refreshed.into_iter().next() {
        let state_lc = refreshed_issue.state.to_lowercase();
        if terminal_states.contains(&state_lc) || !active_states.contains(&state_lc) {
            if let Err(e) = runner.cleanup_workspace(&workspace.path).await {
                warn!(error = %e, "daytona cleanup failed after terminal issue");
            }
        }
    }

    Ok(())
}

fn build_prompt(
    config: &ServiceConfig,
    issue: &Issue,
    attempt: Option<u32>,
) -> Result<String, SympheoError> {
    use liquid::model::Value;
    use std::collections::HashMap;

    let template_str = if config.prompt_template.is_empty() {
        "You are working on an issue from the tracker.".to_string()
    } else {
        config.prompt_template.clone()
    };

    let template = liquid::ParserBuilder::with_stdlib()
        .build()
        .map_err(|e| SympheoError::TemplateParseError(e.to_string()))?
        .parse(&template_str)
        .map_err(|e| SympheoError::TemplateParseError(e.to_string()))?;

    let mut globals = HashMap::new();
    let issue_map = serde_json::to_value(issue).map_err(|e| SympheoError::TemplateRenderError(e.to_string()))?;
    let mut obj = liquid::model::Object::new();
    for (k, v) in issue_map.as_object().unwrap() {
        obj.insert(kstring::KString::from_ref(k), serde_json_to_liquid(v));
    }
    globals.insert("issue".to_string(), Value::Object(obj));
    if let Some(a) = attempt {
        globals.insert("attempt".to_string(), Value::Scalar(a.into()));
    }

    let output = template
        .render(&globals)
        .map_err(|e| SympheoError::TemplateRenderError(e.to_string()))?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_build_prompt_with_template() {
        let mut raw = serde_yaml::Mapping::new();
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
        );
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "Fix {{ issue.title }}".into());
        let issue = Issue {
            id: "1".into(),
            identifier: "TEST-1".into(),
            title: "the bug".into(),
            description: None,
            priority: None,
            state: "todo".into(),
            branch_name: None,
            url: None,
            labels: vec![],
            blocked_by: vec![],
            created_at: None,
            updated_at: None,
        };
        let prompt = build_prompt(&config, &issue, None).unwrap();
        assert_eq!(prompt, "Fix the bug");
    }

    #[test]
    fn test_build_prompt_empty_template() {
        let mut raw = serde_yaml::Mapping::new();
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
        );
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let issue = Issue {
            id: "1".into(),
            identifier: "TEST-1".into(),
            title: "bug".into(),
            description: None,
            priority: None,
            state: "todo".into(),
            branch_name: None,
            url: None,
            labels: vec![],
            blocked_by: vec![],
            created_at: None,
            updated_at: None,
        };
        let prompt = build_prompt(&config, &issue, None).unwrap();
        assert_eq!(prompt, "You are working on an issue from the tracker.");
    }

    #[test]
    fn test_build_prompt_with_attempt() {
        let mut raw = serde_yaml::Mapping::new();
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
        );
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "Attempt {{ attempt }}".into());
        let issue = Issue {
            id: "1".into(),
            identifier: "TEST-1".into(),
            title: "bug".into(),
            description: None,
            priority: None,
            state: "todo".into(),
            branch_name: None,
            url: None,
            labels: vec![],
            blocked_by: vec![],
            created_at: None,
            updated_at: None,
        };
        let prompt = build_prompt(&config, &issue, Some(2)).unwrap();
        assert_eq!(prompt, "Attempt 2");
    }

    #[test]
    fn test_serde_json_to_liquid_null() {
        assert_eq!(
            serde_json_to_liquid(&serde_json::Value::Null),
            liquid::model::Value::Nil
        );
    }

    #[test]
    fn test_serde_json_to_liquid_bool() {
        assert_eq!(
            serde_json_to_liquid(&serde_json::Value::Bool(true)),
            liquid::model::Value::Scalar(true.into())
        );
    }

    #[test]
    fn test_serde_json_to_liquid_number_int() {
        assert_eq!(
            serde_json_to_liquid(&serde_json::Value::Number(42.into())),
            liquid::model::Value::Scalar(42i64.into())
        );
    }

    #[test]
    fn test_serde_json_to_liquid_number_float() {
        assert_eq!(
            serde_json_to_liquid(&serde_json::Value::Number(serde_json::Number::from_f64(2.71).unwrap())),
            liquid::model::Value::Scalar(2.71f64.into())
        );
    }

    #[test]
    fn test_serde_json_to_liquid_string() {
        assert_eq!(
            serde_json_to_liquid(&serde_json::Value::String("hello".into())),
            liquid::model::Value::Scalar("hello".into())
        );
    }

    #[test]
    fn test_serde_json_to_liquid_array() {
        let json = serde_json::json!([1, 2, 3]);
        let liquid = serde_json_to_liquid(&json);
        match liquid {
            liquid::model::Value::Array(arr) => {
                assert_eq!(arr.len(), 3);
            }
            _ => panic!("expected array"),
        }
    }

    #[test]
    fn test_serde_json_to_liquid_object() {
        let json = serde_json::json!({"a": 1, "b": "two"});
        let liquid = serde_json_to_liquid(&json);
        match liquid {
            liquid::model::Value::Object(obj) => {
                assert_eq!(obj.len(), 2);
            }
            _ => panic!("expected object"),
        }
    }

    #[test]
    fn test_build_prompt_unknown_variable_fails() {
        let mut raw = serde_yaml::Mapping::new();
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
        );
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "Hello {{ unknown }}".into());
        let issue = Issue {
            id: "1".into(),
            identifier: "TEST-1".into(),
            title: "bug".into(),
            description: None,
            priority: None,
            state: "todo".into(),
            branch_name: None,
            url: None,
            labels: vec![],
            blocked_by: vec![],
            created_at: None,
            updated_at: None,
        };
        let result = build_prompt(&config, &issue, None);
        assert!(matches!(result, Err(SympheoError::TemplateRenderError(_))));
    }

    #[test]
    fn test_build_prompt_invalid_template_syntax() {
        let mut raw = serde_yaml::Mapping::new();
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
        );
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "{{ unclosed".into());
        let issue = Issue {
            id: "1".into(),
            identifier: "TEST-1".into(),
            title: "bug".into(),
            description: None,
            priority: None,
            state: "todo".into(),
            branch_name: None,
            url: None,
            labels: vec![],
            blocked_by: vec![],
            created_at: None,
            updated_at: None,
        };
        let result = build_prompt(&config, &issue, None);
        assert!(matches!(result, Err(SympheoError::TemplateParseError(_))));
    }
}

fn serde_json_to_liquid(value: &serde_json::Value) -> liquid::model::Value {
    match value {
        serde_json::Value::Null => liquid::model::Value::Nil,
        serde_json::Value::Bool(b) => liquid::model::Value::Scalar((*b).into()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                liquid::model::Value::Scalar(i.into())
            } else if let Some(f) = n.as_f64() {
                liquid::model::Value::Scalar(f.into())
            } else {
                liquid::model::Value::Nil
            }
        }
        serde_json::Value::String(s) => liquid::model::Value::Scalar(s.clone().into()),
        serde_json::Value::Array(arr) => {
            liquid::model::Value::Array(arr.iter().map(serde_json_to_liquid).collect())
        }
        serde_json::Value::Object(obj) => {
            let mut m = liquid::model::Object::new();
            for (k, v) in obj {
                m.insert(kstring::KString::from_ref(k), serde_json_to_liquid(v));
            }
            liquid::model::Value::Object(m)
        }
    }
}
