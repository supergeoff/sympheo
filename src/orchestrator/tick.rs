use crate::agent::parser::AgentEvent;
use crate::agent::runner::AgentRunner;
use crate::config::typed::ServiceConfig;
use crate::error::SympheoError;
use crate::git::adapter::GitStatus;
use crate::orchestrator::retry::schedule_retry;
use crate::orchestrator::state::{OrchestratorState, RunningEntry};
use crate::skills::Skill;
use crate::tracker::IssueTracker;
use crate::tracker::model::{AttemptStatus, Issue, LiveSession, RunAttempt};
use crate::workspace::manager::WorkspaceManager;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
        skills: HashMap<String, Skill>,
        git_adapter: Option<Arc<dyn crate::git::GitAdapter>>,
    ) -> Result<Self, SympheoError> {
        let mut state =
            OrchestratorState::new(config.poll_interval_ms(), config.max_concurrent_agents());
        state.skills = skills;
        let mut workspace_manager = WorkspaceManager::new(&config)?;
        if let Some(adapter) = git_adapter {
            workspace_manager.set_git_adapter(adapter);
        }
        let runner = AgentRunner::new(&config)?;
        Ok(Self {
            state: Arc::new(RwLock::new(state)),
            config: Arc::new(RwLock::new(config)),
            tracker,
            runner: Arc::new(runner),
            workspace_manager: Arc::new(workspace_manager),
        })
    }

    pub async fn reload_config(&self, config: ServiceConfig, skills: HashMap<String, Skill>) {
        let mut state = self.state.write().await;
        state.poll_interval_ms = config.poll_interval_ms();
        state.max_concurrent_agents = config.max_concurrent_agents();
        state.skills = skills;
        *self.config.write().await = config;
    }

    pub async fn tick(&self) {
        info!("orchestrator tick start");
        {
            let mut state = self.state.write().await;
            state.last_tick_at = Some(chrono::Utc::now());
        }

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
                    stagnation_counter: 0,
                    last_state_change_at: Utc::now(),
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
        let stall_timeout_ms = config.cli_stall_timeout_ms();

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
            let id_and_identifier = {
                let state = self.state.read().await;
                state
                    .running
                    .get(&id)
                    .map(|e| (e.issue.id.clone(), e.issue.identifier.clone()))
            };
            self.handle_worker_exit(&id, false, Some("stalled".into()))
                .await;
            if let Some((issue_id, ident)) = id_and_identifier {
                let ws_path = self.workspace_manager.workspace_path(&ident);
                if let Err(e) = self.runner.cleanup_workspace(&ws_path).await {
                    warn!(error = %e, issue_id = %id, "cleanup failed for stalled worker");
                }
                self.workspace_manager
                    .remove_workspace(
                        &ident,
                        &issue_id,
                        config.hook_script("before_remove").as_deref(),
                    )
                    .await;
            }
        }

        // Tracker state refresh
        let refreshed = self.tracker.fetch_issue_states_by_ids(&running_ids).await?;

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
                    let issue_id = entry.issue.id.clone();
                    drop(state);
                    let ws_path = self.workspace_manager.workspace_path(&identifier);
                    if let Err(e) = self.runner.cleanup_workspace(&ws_path).await {
                        warn!(error = %e, "daytona cleanup failed during reconcile");
                    }
                    self.workspace_manager
                        .remove_workspace(
                            &identifier,
                            &issue_id,
                            config.hook_script("before_remove").as_deref(),
                        )
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

    fn spawn_worker(
        &self,
        issue: Issue,
        attempt: Option<u32>,
        max_turns: u32,
        cancelled: Arc<AtomicBool>,
    ) {
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
                st.cli_totals.seconds_running += elapsed;
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
                        if next_attempt > cfg.max_retry_attempts() {
                            warn!(issue_id = %issue.id, attempt = %next_attempt, "max retry attempts reached, dropping retry");
                            st.claimed.remove(&issue.id);
                        } else {
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
            }
        });
    }

    pub async fn handle_worker_exit(&self, issue_id: &str, normal: bool, error: Option<String>) {
        let mut state = self.state.write().await;
        let cfg = self.config.read().await.clone();
        if let Some(entry) = state.running.remove(issue_id) {
            let elapsed = (Utc::now() - entry.started_at).num_seconds() as f64;
            state.cli_totals.seconds_running += elapsed;
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
                if next_attempt > cfg.max_retry_attempts() {
                    warn!(issue_id = %issue_id, attempt = %next_attempt, "max retry attempts reached, dropping retry");
                    state.claimed.remove(issue_id);
                } else {
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
                    if retry.attempt + 1 > config.max_retry_attempts() {
                        warn!(issue_id = %issue_id, attempt = %retry.attempt, "max retry attempts reached, dropping retry");
                        let mut st = self.state.write().await;
                        st.claimed.remove(&issue_id);
                    } else {
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
                    }
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
                        stagnation_counter: 0,
                        last_state_change_at: Utc::now(),
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

async fn apply_agent_event(
    state: &Arc<RwLock<OrchestratorState>>,
    issue_id: &str,
    event: AgentEvent,
) {
    match event {
        AgentEvent::StepStart {
            session_id, part, ..
        } => {
            let mut st = state.write().await;
            if let Some(entry) = st.running.get_mut(issue_id) {
                let new_session_id = format!("{}-{}", session_id, part.message_id);
                if let Some(ref mut sess) = entry.session {
                    sess.session_id = new_session_id;
                    sess.thread_id = session_id.clone();
                    sess.turn_id = part.message_id.clone();
                    sess.last_reported_input_tokens = 0;
                    sess.last_reported_output_tokens = 0;
                    sess.last_reported_total_tokens = 0;
                    sess.last_event = Some("step_start".into());
                    sess.last_timestamp = Some(Utc::now());
                } else {
                    entry.session = Some(LiveSession {
                        session_id: new_session_id,
                        thread_id: session_id.clone(),
                        turn_id: part.message_id.clone(),
                        agent_pid: None,
                        last_event: Some("step_start".into()),
                        last_timestamp: Some(Utc::now()),
                        last_message: None,
                        input_tokens: 0,
                        output_tokens: 0,
                        total_tokens: 0,
                        last_reported_input_tokens: 0,
                        last_reported_output_tokens: 0,
                        last_reported_total_tokens: 0,
                        turn_count: entry.turn_count,
                        pr_url: None,
                    });
                }
            }
        }
        AgentEvent::TokenUsage {
            input,
            output,
            total,
        } => {
            let (last_input, last_output, last_total) = {
                let st = state.read().await;
                st.running
                    .get(issue_id)
                    .and_then(|e| e.session.as_ref())
                    .map(|s| {
                        (
                            s.last_reported_input_tokens,
                            s.last_reported_output_tokens,
                            s.last_reported_total_tokens,
                        )
                    })
                    .unwrap_or((0, 0, 0))
            };

            let delta_input = input.saturating_sub(last_input);
            let delta_output = output.saturating_sub(last_output);
            let delta_total = total.saturating_sub(last_total);

            let mut st = state.write().await;
            st.cli_totals.input_tokens += delta_input;
            st.cli_totals.output_tokens += delta_output;
            st.cli_totals.total_tokens += delta_total;

            if let Some(entry) = st.running.get_mut(issue_id)
                && let Some(ref mut sess) = entry.session
            {
                sess.last_reported_input_tokens = input;
                sess.last_reported_output_tokens = output;
                sess.last_reported_total_tokens = total;
                sess.input_tokens = input;
                sess.output_tokens = output;
                sess.total_tokens = total;
            }
        }
        AgentEvent::RateLimit { payload } => {
            let mut st = state.write().await;
            st.cli_rate_limits = Some(payload);
        }
        AgentEvent::Notification { message, .. }
        | AgentEvent::TurnFailed {
            reason: message, ..
        } => {
            let mut st = state.write().await;
            if let Some(entry) = st.running.get_mut(issue_id)
                && let Some(ref mut sess) = entry.session
            {
                sess.last_event = Some("notification_or_turn_failed".into());
                sess.last_message = Some(message);
                sess.last_timestamp = Some(Utc::now());
            }
        }
        AgentEvent::Text { part, .. } => {
            let mut st = state.write().await;
            if let Some(entry) = st.running.get_mut(issue_id)
                && let Some(ref mut sess) = entry.session
            {
                if let Some(ref mut msg) = sess.last_message {
                    msg.push_str(&part.text);
                } else {
                    sess.last_message = Some(part.text.clone());
                }
            }
        }
        AgentEvent::StepFinish { part, .. } => {
            let mut st = state.write().await;
            if let Some(entry) = st.running.get_mut(issue_id)
                && let Some(ref mut sess) = entry.session
            {
                sess.last_event = Some("step_finish".into());
                sess.last_timestamp = Some(Utc::now());
                if let Some(ref t) = part.tokens {
                    sess.input_tokens = t.input;
                    sess.output_tokens = t.output;
                    sess.total_tokens = t.total;
                }
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)] // Reason: worker function naturally aggregates orchestrator state — splitting would add indirection without clarity.
async fn run_worker(
    mut issue: Issue,
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
            &issue.id,
            config.hook_script("after_create").as_deref(),
        )
        .await?;

    // Auto-create branch when entering in progress
    if issue.state.to_lowercase() == "in progress"
        && issue.branch_name.is_none()
        && let Some(adapter) = workspace_manager.git_adapter()
    {
        let sanitized: String = issue
            .title
            .to_lowercase()
            .replace(|c: char| !c.is_alphanumeric() && c != '-', "-")
            .replace("--", "-")
            .trim_matches('-')
            .chars()
            .take(40)
            .collect();
        let branch = format!("sympheo/{}-{}", issue.id, sanitized);
        let branch = branch.trim_end_matches('-').to_string();
        if let Err(e) = adapter
            .checkout_branch(&workspace.path, &branch, true)
            .await
        {
            warn!(issue_id = %issue.id, error = %e, "failed to create branch");
        } else {
            if let Err(e) = adapter.push(&workspace.path, "origin", &branch).await {
                warn!(issue_id = %issue.id, error = %e, "failed to push branch");
            }
            issue.branch_name = Some(branch);
        }
    }

    let mut attempt_record = RunAttempt::new(
        issue.id.clone(),
        issue.identifier.clone(),
        attempt,
        workspace.path.clone(),
    );

    if let Some(script) = config.hook_script("before_run") {
        attempt_record.transition(AttemptStatus::PreparingWorkspace);
        let env = crate::workspace::manager::sympheo_hook_env(
            &issue.identifier,
            &issue.id,
            &workspace.path,
        );
        workspace_manager
            .run_hook("before_run", &script, &workspace.path, &env)
            .await?;
    }

    let mut current_session: Option<String> = None;
    let mut turn_number = 0;

    let skills = {
        let st = state.read().await;
        st.skills.clone()
    };

    loop {
        if cancelled.load(Ordering::Relaxed) {
            info!(issue_id = %issue.id, "worker cancelled by orchestrator, stopping");
            break;
        }

        // Git state verification before each turn
        if let Some(adapter) = workspace_manager.git_adapter() {
            if let Err(e) = adapter.fetch(&workspace.path, "origin").await {
                warn!(issue_id = %issue.id, error = %e, "git fetch failed");
            }
            match adapter.status(&workspace.path).await {
                Ok(GitStatus::Clean) => {}
                Ok(GitStatus::DetachedHead) | Ok(GitStatus::Dirty(_)) => {
                    let target = issue.branch_name.as_deref().unwrap_or("origin/main");
                    if let Err(e) = adapter.reset_hard(&workspace.path, target).await {
                        warn!(issue_id = %issue.id, error = %e, target = %target, "git reset failed");
                    } else {
                        info!(issue_id = %issue.id, target = %target, "reset workspace to clean state");
                    }
                }
                Err(e) => {
                    warn!(issue_id = %issue.id, error = %e, "git status failed");
                }
            }
        }

        turn_number += 1;

        {
            let mut st = state.write().await;
            if let Some(entry) = st.running.get_mut(&issue.id) {
                entry.turn_count = turn_number;
            }
        }

        attempt_record.transition(AttemptStatus::BuildingPrompt);
        let skill_content = skills
            .get(&issue.state.to_lowercase())
            .or_else(|| skills.get("default"))
            .map(|s| s.content.as_str());

        let prompt = if turn_number == 1 {
            build_prompt_strict(config, &issue, attempt, skill_content)?
        } else {
            config.continuation_prompt()
        };

        attempt_record.transition(AttemptStatus::LaunchingAgentProcess);

        // Channel + consumer must exist BEFORE the turn starts so events
        // update orchestrator state in real time. Sized for chatty turns
        // (text deltas, tool calls) without back-pressuring the backend
        // stdout reader during normal operation.
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(1024);

        let state_for_events = state.clone();
        let issue_id_for_events = issue.id.clone();
        let event_consumer = tokio::spawn(async move {
            let mut rx = event_rx;
            while let Some(event) = rx.recv().await {
                apply_agent_event(&state_for_events, &issue_id_for_events, event).await;
            }
        });

        attempt_record.transition(AttemptStatus::StreamingTurn);
        let turn_result = runner
            .run_turn(
                &issue,
                &prompt,
                current_session.as_deref(),
                &workspace.path,
                cancelled.clone(),
                event_tx,
            )
            .await?;

        // run_turn dropped its sender on return — let the consumer drain
        // the remaining events queued in the channel before we proceed.
        let _ = event_consumer.await;

        // Fallback: if no TokenUsage event was received, use tokens from turn_result
        let already_counted = {
            let st = state.read().await;
            st.running
                .get(&issue.id)
                .and_then(|e| e.session.as_ref())
                .map(|s| s.last_reported_total_tokens > 0)
                .unwrap_or(false)
        };

        if !already_counted && let Some(ref token_info) = turn_result.tokens {
            let mut st = state.write().await;
            st.cli_totals.input_tokens += token_info.input;
            st.cli_totals.output_tokens += token_info.output;
            st.cli_totals.total_tokens += token_info.total;
            if let Some(entry) = st.running.get_mut(&issue.id)
                && let Some(ref mut sess) = entry.session
            {
                sess.input_tokens = token_info.input;
                sess.output_tokens = token_info.output;
                sess.total_tokens = token_info.total;
                sess.last_reported_input_tokens = token_info.input;
                sess.last_reported_output_tokens = token_info.output;
                sess.last_reported_total_tokens = token_info.total;
            }
        }

        // Update session metadata from turn result
        {
            let mut st = state.write().await;
            if let Some(entry) = st.running.get_mut(&issue.id) {
                if let Some(ref mut sess) = entry.session {
                    sess.session_id = format!("{}-{}", turn_result.session_id, turn_result.turn_id);
                    sess.thread_id = turn_result.session_id.clone();
                    sess.turn_id = turn_result.turn_id.clone();
                    sess.last_event = Some("turn_completed".into());
                    sess.last_timestamp = Some(Utc::now());
                    sess.last_message = Some(turn_result.text.clone());
                    if let Some(ref t) = turn_result.tokens {
                        sess.input_tokens = t.input;
                        sess.output_tokens = t.output;
                        sess.total_tokens = t.total;
                    }
                    sess.turn_count = entry.turn_count;
                } else {
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
                        last_reported_input_tokens: turn_result
                            .tokens
                            .as_ref()
                            .map(|t| t.input)
                            .unwrap_or(0),
                        last_reported_output_tokens: turn_result
                            .tokens
                            .as_ref()
                            .map(|t| t.output)
                            .unwrap_or(0),
                        last_reported_total_tokens: turn_result
                            .tokens
                            .as_ref()
                            .map(|t| t.total)
                            .unwrap_or(0),
                        turn_count: entry.turn_count,
                        pr_url: None,
                    });
                }
            }
        }

        if !turn_result.success {
            attempt_record.transition(AttemptStatus::Failed);
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

            let prev_state = issue.state.to_lowercase();
            {
                let mut st = state.write().await;
                if let Some(entry) = st.running.get_mut(&issue.id) {
                    if state_lc == prev_state {
                        entry.stagnation_counter += 1;
                        if entry.stagnation_counter >= 3 {
                            warn!(issue_id = %issue.id, "issue stagnant for 3 turns, forcing stop");
                            return Err(SympheoError::AgentRunnerError(
                                "stagnation guardrail triggered".into(),
                            ));
                        }
                    } else {
                        entry.stagnation_counter = 0;
                        entry.last_state_change_at = Utc::now();
                        issue.state = refreshed_issue.state.clone();
                    }
                }
            }

            let max_per_state = config.max_turns_per_state().get(&state_lc).copied();
            if let Some(max_state_turns) = max_per_state
                && turn_number >= max_state_turns
            {
                info!(issue_id = %issue.id, state = %state_lc, max = %max_state_turns, "max turns for state reached");
                break;
            }
        }

        if turn_number >= max_turns {
            break;
        }
    }

    attempt_record.transition(AttemptStatus::Finishing);
    if let Some(script) = config.hook_script("after_run") {
        let env = crate::workspace::manager::sympheo_hook_env(
            &issue.identifier,
            &issue.id,
            &workspace.path,
        );
        // SPEC §9.4: after_run failure is logged and ignored.
        if let Err(e) = workspace_manager
            .run_hook("after_run", &script, &workspace.path, &env)
            .await
        {
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
        if (terminal_states.contains(&state_lc) || !active_states.contains(&state_lc))
            && let Err(e) = runner.cleanup_workspace(&workspace.path).await
        {
            warn!(error = %e, "daytona cleanup failed after terminal issue");
        }
    }

    Ok(())
}

fn build_prompt_strict(
    config: &ServiceConfig,
    issue: &Issue,
    attempt: Option<u32>,
    skill_instructions: Option<&str>,
) -> Result<String, SympheoError> {
    use liquid::model::Value;
    use std::collections::HashMap;

    let template_str = if config.prompt_template.is_empty() {
        "You are working on an issue from the tracker.".to_string()
    } else {
        config.prompt_template.clone()
    };

    // Strict mode: validate root variables
    let re = regex::Regex::new(r"\{\{\s*([a-zA-Z_][a-zA-Z0-9_]*)\s*\}\}")
        .map_err(|e| SympheoError::TemplateParseError(e.to_string()))?;
    let available_vars = ["issue", "attempt"];
    for cap in re.captures_iter(&template_str) {
        let var_name = cap.get(1).unwrap().as_str();
        if !available_vars.contains(&var_name) {
            return Err(SympheoError::TemplateRenderError(format!(
                "Unknown variable: {}",
                var_name
            )));
        }
    }

    let template = liquid::ParserBuilder::with_stdlib()
        .build()
        .map_err(|e| SympheoError::TemplateParseError(e.to_string()))?
        .parse(&template_str)
        .map_err(|e| SympheoError::TemplateParseError(e.to_string()))?;

    let mut globals = HashMap::new();
    let issue_map = serde_json::to_value(issue)
        .map_err(|e| SympheoError::TemplateRenderError(e.to_string()))?;
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

    let output = match skill_instructions {
        Some(instr) if !instr.trim().is_empty() => {
            format!("{}\n\n---\n\n{}", instr.trim(), output)
        }
        _ => output,
    };

    Ok(output)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_build_prompt_with_template() {
        let mut raw = serde_json::Map::new();
        raw.insert(
            "tracker".into(),
            serde_json::Value::Object(serde_json::Map::new()),
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
            ..Default::default()
        };
        let prompt = build_prompt_strict(&config, &issue, None, None).unwrap();
        assert_eq!(prompt, "Fix the bug");
    }

    #[test]
    fn test_build_prompt_empty_template() {
        let mut raw = serde_json::Map::new();
        raw.insert(
            "tracker".into(),
            serde_json::Value::Object(serde_json::Map::new()),
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
            ..Default::default()
        };
        let prompt = build_prompt_strict(&config, &issue, None, None).unwrap();
        assert_eq!(prompt, "You are working on an issue from the tracker.");
    }

    #[test]
    fn test_build_prompt_with_attempt() {
        let mut raw = serde_json::Map::new();
        raw.insert(
            "tracker".into(),
            serde_json::Value::Object(serde_json::Map::new()),
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
            ..Default::default()
        };
        let prompt = build_prompt_strict(&config, &issue, Some(2), None).unwrap();
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
            serde_json_to_liquid(&serde_json::Value::Number(
                serde_json::Number::from_f64(std::f64::consts::PI).unwrap()
            )),
            liquid::model::Value::Scalar(std::f64::consts::PI.into())
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
        let mut raw = serde_json::Map::new();
        raw.insert(
            "tracker".into(),
            serde_json::Value::Object(serde_json::Map::new()),
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
            ..Default::default()
        };
        let result = build_prompt_strict(&config, &issue, None, None);
        assert!(matches!(result, Err(SympheoError::TemplateRenderError(_))));
    }

    #[test]
    fn test_build_prompt_strict_unknown_root_var() {
        let mut raw = serde_json::Map::new();
        raw.insert(
            "tracker".into(),
            serde_json::Value::Object(serde_json::Map::new()),
        );
        let config =
            ServiceConfig::new(raw, PathBuf::from("/tmp"), "Hello {{ unknown_var }}".into());
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
            ..Default::default()
        };
        let result = build_prompt_strict(&config, &issue, None, None);
        assert!(matches!(result, Err(SympheoError::TemplateRenderError(_))));
    }

    #[test]
    fn test_build_prompt_invalid_template_syntax() {
        let mut raw = serde_json::Map::new();
        raw.insert(
            "tracker".into(),
            serde_json::Value::Object(serde_json::Map::new()),
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
            ..Default::default()
        };
        let result = build_prompt_strict(&config, &issue, None, None);
        assert!(matches!(result, Err(SympheoError::TemplateParseError(_))));
    }

    #[test]
    fn test_build_prompt_with_skill() {
        let mut raw = serde_json::Map::new();
        raw.insert(
            "tracker".into(),
            serde_json::Value::Object(serde_json::Map::new()),
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
            ..Default::default()
        };
        let prompt =
            build_prompt_strict(&config, &issue, None, Some("Analyze the issue first.")).unwrap();
        assert!(prompt.contains("Analyze the issue first."));
        assert!(prompt.contains("Fix the bug"));
        assert!(prompt.contains("---"));
    }

    #[tokio::test]
    async fn test_apply_agent_event_step_start_creates_session() {
        use crate::agent::parser::{AgentEvent, StepStartPart};
        use crate::orchestrator::state::{OrchestratorState, RunningEntry};
        use crate::tracker::model::Issue;
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;

        let state = Arc::new(RwLock::new(OrchestratorState::new(5000, 5)));
        {
            let mut st = state.write().await;
            st.running.insert(
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
                        ..Default::default()
                    },
                    session: None,
                    started_at: chrono::Utc::now(),
                    retry_attempt: None,
                    turn_count: 2,
                    stagnation_counter: 0,
                    last_state_change_at: chrono::Utc::now(),
                    cancelled: Arc::new(AtomicBool::new(false)),
                },
            );
        }

        let event = AgentEvent::StepStart {
            timestamp: 0,
            session_id: "sess-1".into(),
            part: StepStartPart {
                id: "p1".into(),
                message_id: "m1".into(),
                session_id: "sess-1".into(),
                part_type: "step_start".into(),
            },
        };

        apply_agent_event(&state, "1", event).await;

        let st = state.read().await;
        let entry = st.running.get("1").unwrap();
        let sess = entry.session.as_ref().unwrap();
        assert_eq!(sess.session_id, "sess-1-m1");
        assert_eq!(sess.thread_id, "sess-1");
        assert_eq!(sess.turn_id, "m1");
        assert_eq!(sess.turn_count, 2);
    }

    #[tokio::test]
    async fn test_apply_agent_event_step_start_updates_session() {
        use crate::agent::parser::{AgentEvent, StepStartPart};
        use crate::orchestrator::state::{OrchestratorState, RunningEntry};
        use crate::tracker::model::{Issue, LiveSession};
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;

        let state = Arc::new(RwLock::new(OrchestratorState::new(5000, 5)));
        {
            let mut st = state.write().await;
            st.running.insert(
                "1".into(),
                RunningEntry {
                    issue: Issue::default(),
                    session: Some(LiveSession {
                        session_id: "old".into(),
                        thread_id: "old-t".into(),
                        turn_id: "old-u".into(),
                        agent_pid: None,
                        last_event: None,
                        last_message: None,
                        last_timestamp: None,
                        input_tokens: 100,
                        output_tokens: 100,
                        total_tokens: 100,
                        last_reported_input_tokens: 50,
                        last_reported_output_tokens: 50,
                        last_reported_total_tokens: 50,
                        turn_count: 0,
                        pr_url: None,
                    }),
                    started_at: chrono::Utc::now(),
                    retry_attempt: None,
                    turn_count: 0,
                    stagnation_counter: 0,
                    last_state_change_at: chrono::Utc::now(),
                    cancelled: Arc::new(AtomicBool::new(false)),
                },
            );
        }

        let event = AgentEvent::StepStart {
            timestamp: 0,
            session_id: "sess-2".into(),
            part: StepStartPart {
                id: "p1".into(),
                message_id: "m2".into(),
                session_id: "sess-2".into(),
                part_type: "step_start".into(),
            },
        };

        apply_agent_event(&state, "1", event).await;

        let st = state.read().await;
        let sess = st.running.get("1").unwrap().session.as_ref().unwrap();
        assert_eq!(sess.session_id, "sess-2-m2");
        assert_eq!(sess.thread_id, "sess-2");
        assert_eq!(sess.turn_id, "m2");
        assert_eq!(sess.last_reported_input_tokens, 0);
        assert_eq!(sess.last_reported_output_tokens, 0);
        assert_eq!(sess.last_reported_total_tokens, 0);
        assert_eq!(sess.last_event, Some("step_start".into()));
    }

    #[tokio::test]
    async fn test_apply_agent_event_token_usage_updates_totals() {
        use crate::agent::parser::AgentEvent;
        use crate::orchestrator::state::{OrchestratorState, RunningEntry};
        use crate::tracker::model::{Issue, LiveSession};
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;

        let state = Arc::new(RwLock::new(OrchestratorState::new(5000, 5)));
        {
            let mut st = state.write().await;
            st.running.insert(
                "1".into(),
                RunningEntry {
                    issue: Issue::default(),
                    session: Some(LiveSession {
                        session_id: "s1".into(),
                        thread_id: "t1".into(),
                        turn_id: "u1".into(),
                        agent_pid: None,
                        last_event: None,
                        last_message: None,
                        last_timestamp: None,
                        input_tokens: 0,
                        output_tokens: 0,
                        total_tokens: 0,
                        last_reported_input_tokens: 10,
                        last_reported_output_tokens: 20,
                        last_reported_total_tokens: 30,
                        turn_count: 0,
                        pr_url: None,
                    }),
                    started_at: chrono::Utc::now(),
                    retry_attempt: None,
                    turn_count: 0,
                    stagnation_counter: 0,
                    last_state_change_at: chrono::Utc::now(),
                    cancelled: Arc::new(AtomicBool::new(false)),
                },
            );
        }

        let event = AgentEvent::TokenUsage {
            input: 50,
            output: 80,
            total: 130,
        };
        apply_agent_event(&state, "1", event).await;

        let st = state.read().await;
        assert_eq!(st.cli_totals.input_tokens, 40); // 50 - 10
        assert_eq!(st.cli_totals.output_tokens, 60); // 80 - 20
        assert_eq!(st.cli_totals.total_tokens, 100); // 130 - 30
        let sess = st.running.get("1").unwrap().session.as_ref().unwrap();
        assert_eq!(sess.input_tokens, 50);
        assert_eq!(sess.output_tokens, 80);
        assert_eq!(sess.total_tokens, 130);
    }

    #[tokio::test]
    async fn test_apply_agent_event_rate_limit() {
        use crate::agent::parser::AgentEvent;
        use crate::orchestrator::state::OrchestratorState;

        let state = Arc::new(RwLock::new(OrchestratorState::new(5000, 5)));
        let event = AgentEvent::RateLimit {
            payload: serde_json::json!({"limit": 100}),
        };
        apply_agent_event(&state, "1", event).await;

        let st = state.read().await;
        assert_eq!(st.cli_rate_limits, Some(serde_json::json!({"limit": 100})));
    }

    #[tokio::test]
    async fn test_apply_agent_event_notification() {
        use crate::agent::parser::AgentEvent;
        use crate::orchestrator::state::{OrchestratorState, RunningEntry};
        use crate::tracker::model::{Issue, LiveSession};
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;

        let state = Arc::new(RwLock::new(OrchestratorState::new(5000, 5)));
        {
            let mut st = state.write().await;
            st.running.insert(
                "1".into(),
                RunningEntry {
                    issue: Issue::default(),
                    session: Some(LiveSession {
                        session_id: "s1".into(),
                        thread_id: "t1".into(),
                        turn_id: "u1".into(),
                        agent_pid: None,
                        last_event: None,
                        last_message: None,
                        last_timestamp: None,
                        input_tokens: 0,
                        output_tokens: 0,
                        total_tokens: 0,
                        last_reported_input_tokens: 0,
                        last_reported_output_tokens: 0,
                        last_reported_total_tokens: 0,
                        turn_count: 0,
                        pr_url: None,
                    }),
                    started_at: chrono::Utc::now(),
                    retry_attempt: None,
                    turn_count: 0,
                    stagnation_counter: 0,
                    last_state_change_at: chrono::Utc::now(),
                    cancelled: Arc::new(AtomicBool::new(false)),
                },
            );
        }

        let event = AgentEvent::Notification {
            session_id: "s1".into(),
            message: "hello world".into(),
        };
        apply_agent_event(&state, "1", event).await;

        let st = state.read().await;
        let sess = st.running.get("1").unwrap().session.as_ref().unwrap();
        assert_eq!(sess.last_message, Some("hello world".into()));
    }

    #[tokio::test]
    async fn test_apply_agent_event_text_appends() {
        use crate::agent::parser::{AgentEvent, TextPart};
        use crate::orchestrator::state::{OrchestratorState, RunningEntry};
        use crate::tracker::model::{Issue, LiveSession};
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;

        let state = Arc::new(RwLock::new(OrchestratorState::new(5000, 5)));
        {
            let mut st = state.write().await;
            st.running.insert(
                "1".into(),
                RunningEntry {
                    issue: Issue::default(),
                    session: Some(LiveSession {
                        session_id: "s1".into(),
                        thread_id: "t1".into(),
                        turn_id: "u1".into(),
                        agent_pid: None,
                        last_event: None,
                        last_message: Some("pre ".into()),
                        last_timestamp: None,
                        input_tokens: 0,
                        output_tokens: 0,
                        total_tokens: 0,
                        last_reported_input_tokens: 0,
                        last_reported_output_tokens: 0,
                        last_reported_total_tokens: 0,
                        turn_count: 0,
                        pr_url: None,
                    }),
                    started_at: chrono::Utc::now(),
                    retry_attempt: None,
                    turn_count: 0,
                    stagnation_counter: 0,
                    last_state_change_at: chrono::Utc::now(),
                    cancelled: Arc::new(AtomicBool::new(false)),
                },
            );
        }

        let event = AgentEvent::Text {
            timestamp: 0,
            session_id: "s1".into(),
            part: TextPart {
                id: "p1".into(),
                message_id: "m1".into(),
                session_id: "s1".into(),
                part_type: "text".into(),
                text: "fix".into(),
                time: None,
            },
        };
        apply_agent_event(&state, "1", event).await;

        let st = state.read().await;
        let sess = st.running.get("1").unwrap().session.as_ref().unwrap();
        assert_eq!(sess.last_message, Some("pre fix".into()));
    }

    #[tokio::test]
    async fn test_apply_agent_event_step_finish_with_tokens() {
        use crate::agent::parser::{AgentEvent, StepFinishPart, TokenInfo};
        use crate::orchestrator::state::{OrchestratorState, RunningEntry};
        use crate::tracker::model::{Issue, LiveSession};
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;

        let state = Arc::new(RwLock::new(OrchestratorState::new(5000, 5)));
        {
            let mut st = state.write().await;
            st.running.insert(
                "1".into(),
                RunningEntry {
                    issue: Issue::default(),
                    session: Some(LiveSession {
                        session_id: "s1".into(),
                        thread_id: "t1".into(),
                        turn_id: "u1".into(),
                        agent_pid: None,
                        last_event: None,
                        last_message: None,
                        last_timestamp: None,
                        input_tokens: 0,
                        output_tokens: 0,
                        total_tokens: 0,
                        last_reported_input_tokens: 0,
                        last_reported_output_tokens: 0,
                        last_reported_total_tokens: 0,
                        turn_count: 0,
                        pr_url: None,
                    }),
                    started_at: chrono::Utc::now(),
                    retry_attempt: None,
                    turn_count: 0,
                    stagnation_counter: 0,
                    last_state_change_at: chrono::Utc::now(),
                    cancelled: Arc::new(AtomicBool::new(false)),
                },
            );
        }

        let event = AgentEvent::StepFinish {
            timestamp: 0,
            session_id: "s1".into(),
            part: StepFinishPart {
                id: "p1".into(),
                reason: "done".into(),
                message_id: "m1".into(),
                session_id: "s1".into(),
                part_type: "step_finish".into(),
                tokens: Some(TokenInfo {
                    total: 100,
                    input: 40,
                    output: 60,
                    reasoning: 0,
                    cache: None,
                }),
                cost: None,
            },
        };
        apply_agent_event(&state, "1", event).await;

        let st = state.read().await;
        let sess = st.running.get("1").unwrap().session.as_ref().unwrap();
        assert_eq!(sess.last_event, Some("step_finish".into()));
        assert_eq!(sess.input_tokens, 40);
        assert_eq!(sess.output_tokens, 60);
        assert_eq!(sess.total_tokens, 100);
    }
}
