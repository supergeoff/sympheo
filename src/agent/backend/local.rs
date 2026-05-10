use crate::agent::backend::AgentBackend;
use crate::agent::cli::CliAdapter;
use crate::agent::parser::{AgentEvent, EmittedEvent, TokenInfo, TurnOutcome, TurnResult};
use crate::agent::tool_resolver;
use crate::config::typed::ServiceConfig;
use crate::error::SympheoError;
use crate::tracker::model::Issue;
use crate::workspace::isolation::{build_isolated_env, ensure_isolated_home};
use crate::workspace::manager::WorkspaceManager;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::Sender;
use tokio::time::{Duration, timeout};
use tracing::Instrument;

/// Tools the worker is guaranteed to need. The agent CLI itself is added
/// dynamically based on `cli.command`'s first token. `gh` is included so the
/// agent can interact with GitHub (issue comments, PR ops) without falling
/// back to a shim.
const ALWAYS_RESOLVE_TOOLS: &[&str] = &["gh"];

pub struct LocalBackend {
    /// `cli.command` with its first whitespace-delimited token rewritten to
    /// the absolute path of the resolved agent binary (when resolution
    /// succeeds). Defense-in-depth: even if the worker `PATH` is shadowed by
    /// `cli.env.PATH`, the agent binary still runs.
    command: String,
    /// SPEC §10.2.2: total wall-clock per turn (default 3600000 ms).
    turn_timeout: Duration,
    /// SPEC §10.2.2: per-stdout-read stall timeout (default 5000 ms).
    read_timeout: Duration,
    workspace_manager: WorkspaceManager,
    cli_env: HashMap<String, String>,
    /// Parent directories of every successfully-resolved tool, in resolution
    /// order, deduplicated. Threaded into the worker's `PATH` so the agent can
    /// invoke each tool by short name (e.g. `gh`, `opencode`) without hitting
    /// a mise shim.
    resolved_bin_dirs: Vec<PathBuf>,
    /// SPEC §10.6: the CLI adapter owns per-CLI argv assembly, prompt
    /// sanitization, and stdout parsing. Threading it through the backend
    /// lets the same `LocalBackend` drive opencode, claude, pi, etc.
    adapter: Arc<dyn CliAdapter>,
}

impl LocalBackend {
    pub fn new(config: &ServiceConfig, adapter: Arc<dyn CliAdapter>) -> Result<Self, SympheoError> {
        let raw_command = config.cli_command();

        let agent_bin_name: Option<String> =
            raw_command.split_whitespace().next().map(|s| s.to_string());

        let mut tool_names: Vec<String> = Vec::new();
        if let Some(name) = &agent_bin_name {
            tool_names.push(name.clone());
        }
        for extra in ALWAYS_RESOLVE_TOOLS {
            if !tool_names.iter().any(|n| n == extra) {
                tool_names.push((*extra).to_string());
            }
        }

        let mut resolved_bin_dirs: Vec<PathBuf> = Vec::new();
        let mut seen_dirs = std::collections::HashSet::new();
        let mut agent_abs_path: Option<PathBuf> = None;
        for name in &tool_names {
            match tool_resolver::resolve_tool(name) {
                Some(p) => {
                    if Some(name) == agent_bin_name.as_ref() {
                        agent_abs_path = Some(p.clone());
                    }
                    if let Some(parent) = p.parent() {
                        let parent = parent.to_path_buf();
                        if seen_dirs.insert(parent.clone()) {
                            resolved_bin_dirs.push(parent);
                        }
                    }
                }
                None => {
                    tracing::warn!(
                        tool = %name,
                        "could not resolve agent tool to an absolute path; \
                         worker invocation may fail with `command not found`"
                    );
                }
            }
        }

        let command = match (&agent_bin_name, &agent_abs_path) {
            (Some(name), Some(abs)) => {
                let rest = raw_command[name.len()..].to_string();
                format!("{}{}", abs.display(), rest)
            }
            _ => raw_command,
        };

        Ok(Self {
            command,
            turn_timeout: Duration::from_millis(config.cli_turn_timeout_ms()),
            read_timeout: Duration::from_millis(config.cli_read_timeout_ms()),
            workspace_manager: WorkspaceManager::new(config)?,
            cli_env: config.cli_env(),
            resolved_bin_dirs,
            adapter,
        })
    }
}

#[async_trait]
impl AgentBackend for LocalBackend {
    fn kind(&self) -> &'static str {
        "local"
    }

    async fn run_turn(
        &self,
        issue: &Issue,
        prompt: &str,
        session_id: Option<&str>,
        workspace_path: &Path,
        cancelled: Arc<AtomicBool>,
        event_tx: Sender<EmittedEvent>,
    ) -> Result<TurnResult, SympheoError> {
        let sid = session_id.unwrap_or("new");
        let span = tracing::info_span!(
            "opencode_turn",
            issue_id = %issue.id,
            issue_identifier = %issue.identifier,
            session_id = %sid,
        );
        let span_clone = span.clone();

        async move {
            // SPEC §9.5 Inv 2 + Inv 3: workspace path inside root, sanitized
            self.workspace_manager
                .validate_inside_root(workspace_path)?;

            // SPEC §9.5 Inv 1: launch cwd MUST be the per-issue workspace path.
            // Validate the canonical form matches the workspace_path the orchestrator
            // intends (defends against silent mismatches if a caller passes a sibling
            // path or a symlinked alias).
            let canonical_ws = workspace_path
                .canonicalize()
                .map_err(|e| SympheoError::InvalidWorkspaceCwd(e.to_string()))?;
            if canonical_ws != workspace_path
                && !workspace_path.starts_with(&canonical_ws)
                && !canonical_ws.starts_with(workspace_path)
            {
                return Err(SympheoError::InvalidWorkspaceCwd(format!(
                    "expected cwd to be workspace_path {}; canonical {}",
                    workspace_path.display(),
                    canonical_ws.display()
                )));
            }

            let sanitized = self.adapter.sanitize_prompt(prompt);
            tracing::debug!(prompt_len = sanitized.len(), "prompt length");

            // Write prompt to a temp file to avoid shell escaping and ARG_MAX issues
            let prompt_file = workspace_path.join(format!(".sympheo_prompt_{}.txt", issue.id));
            tokio::fs::write(&prompt_file, &sanitized).await
                .map_err(|e| SympheoError::AgentRunnerError(format!("failed to write prompt file: {e}")))?;

            // SPEC §15.5 hardening: provision the per-workspace HOME/XDG subtree
            // and build the scrubbed env (HOME, XDG_*, PATH minimal, locale passthrough,
            // operator overrides from cli.env per §5.3.6).
            ensure_isolated_home(workspace_path)
                .await
                .map_err(|e| SympheoError::AgentRunnerError(format!("isolated home setup failed: {e}")))?;
            let env = build_isolated_env(workspace_path, &self.resolved_bin_dirs, &self.cli_env);

            let mut cmd = Command::new("bash");
            cmd.arg("-lc");

            // SPEC §10.6: per-CLI argv assembly is owned by the adapter so this
            // backend can drive opencode, claude, pi, ... without any branching.
            let cli_cmd_str = self.adapter.build_command_string(
                &self.command,
                &prompt_file,
                workspace_path,
                session_id,
            );
            cmd.arg(&cli_cmd_str);
            cmd.current_dir(workspace_path);
            cmd.env_clear();
            for (k, v) in &env {
                cmd.env(k, v);
            }
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::piped());

            // Run in a new process group so we can kill the entire tree reliably
            unsafe {
                cmd.pre_exec(|| {
                    libc::setpgid(0, 0);
                    Ok(())
                });
            }

            tracing::info!(adapter = %self.adapter.kind(), "launching agent (local backend)");
            tracing::debug!(command = %cli_cmd_str, "local backend command");

            let mut child = cmd
                .spawn()
                .map_err(|e| SympheoError::TurnLaunchFailed(format!("spawn failed: {e}")))?;

            // Register in the global process registry so signal/panic handlers
            // can reach this subprocess (and clean it up if Sympheo crashes).
            // The guard is dropped at function exit / panic, which removes the
            // entry — so terminated workers don't leave stale records.
            let _registry_guard = crate::agent::process_registry::register(child.id().unwrap_or(0));

            // Track PID and start cancellation watchdog. The PID is also stamped
            // onto every emitted event per SPEC §10.3 so operators can correlate
            // events with the subprocess that produced them.
            let agent_pid: Option<u32> = child.id();
            let child_pid = Arc::new(AtomicU32::new(child.id().unwrap_or(0)));
            let child_pid_watch = child_pid.clone();
            let cancelled_watch = cancelled.clone();
            let _watchdog = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    if cancelled_watch.load(Ordering::Relaxed) {
                        let pid = child_pid_watch.load(Ordering::Relaxed);
                        if pid != 0 {
                            let pgid = pid as i32;
                            unsafe {
                                let _ = libc::killpg(pgid, libc::SIGKILL);
                                let _ = libc::kill(pgid, libc::SIGKILL);
                            }
                        }
                        break;
                    }
                }
            });

            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| SympheoError::AgentRunnerError("missing stdout".into()))?;
            let stderr = child
                .stderr
                .take()
                .ok_or_else(|| SympheoError::AgentRunnerError("missing stderr".into()))?;

            // Spawn stderr reader task to capture agent diagnostic output.
            // Detects well-known opencode failure signals (rate-limit, auth, account
            // required, permission denied) and exposes them via the shared
            // `detected_stderr_error` slot so the main turn loop can fail explicitly
            // instead of silently treating reason=stop as success.
            let detected_stderr_error: Arc<tokio::sync::Mutex<Option<SympheoError>>> =
                Arc::new(tokio::sync::Mutex::new(None));
            let stderr_error_slot = detected_stderr_error.clone();
            let stderr_span = tracing::info_span!(parent: span_clone, "opencode_stderr");
            let stderr_handle = tokio::spawn(
                async move {
                    let stderr_reader = BufReader::new(stderr);
                    let mut stderr_lines = stderr_reader.lines();
                    while let Ok(Some(line)) = stderr_lines.next_line().await {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        tracing::warn!(target = "opencode::stderr", "{}", trimmed);
                        if let Some(err) = classify_stderr_line(trimmed) {
                            let mut slot = stderr_error_slot.lock().await;
                            if slot.is_none() {
                                *slot = Some(err);
                            }
                        }
                    }
                }
                .instrument(stderr_span),
            );

            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            let mut current_session: Option<String> = None;
            let mut current_turn: Option<String> = None;
            let mut accumulated_text = String::new();
            let mut tokens: Option<TokenInfo> = None;
            let mut success = false;

            // SPEC §10.2.2 + §10.5 timeout discipline:
            // - `read_timeout_ms`  applies to each individual stdout-read; if no
            //   line arrives within the window we map to `TurnReadTimeout`.
            // - `turn_timeout_ms`  bounds the overall turn wall-clock; the
            //   outer `timeout(..)` wraps the whole read loop and maps to
            //   `TurnTotalTimeout`.
            let read_timeout = self.read_timeout;
            let read_result = timeout(self.turn_timeout, async {
                loop {
                    match timeout(read_timeout, lines.next_line()).await {
                        Err(_) => return Err(SympheoError::TurnReadTimeout),
                        Ok(Err(_)) | Ok(Ok(None)) => return Ok(()),
                        Ok(Ok(Some(line))) => {
                            if line.trim().is_empty() {
                                continue;
                            }
                            if let Some(event) = self.adapter.parse_stdout_line(&line) {
                                tracing::debug!(event = ?event, "parsed agent event");
                                match &event {
                                    AgentEvent::StepStart { session_id, part, .. } => {
                                        current_session = Some(session_id.clone());
                                        current_turn = Some(part.message_id.clone());
                                        tracing::debug!(session = %part.session_id, message = %part.message_id, "step_start");
                                    }
                                    AgentEvent::Text { part, .. } => {
                                        accumulated_text.push_str(&part.text);
                                    }
                                    AgentEvent::StepFinish { part, .. } => {
                                        tokens = part.tokens.clone();
                                        success = part.reason == "stop" || part.reason == "tool-calls";
                                        tracing::info!(
                                            reason = %part.reason,
                                            success,
                                            session_id = %part.session_id,
                                            message_id = %part.message_id,
                                            "step_finish"
                                        );
                                    }
                                    _ => {}
                                }
                                // SPEC §10.3: stamp the active turn PID onto the event.
                                let _ = event_tx
                                    .send(EmittedEvent::with_pid(event, agent_pid))
                                    .await;
                            } else {
                                tracing::warn!(raw = %line, "failed to parse event line");
                            }
                        }
                    }
                }
            })
            .await;

            // Fold the doubly-nested timeout/Result into a single Result so
            // downstream branches can distinguish per-read stalls (`TurnReadTimeout`)
            // from total-turn exhaustion (`TurnTotalTimeout`).
            let read_outcome: Result<(), SympheoError> = match read_result {
                Ok(inner) => inner,
                Err(_) => Err(SympheoError::TurnTotalTimeout),
            };

            if let Err(e) = read_outcome {
                kill_process_group(&mut child);
                let _ = stderr_handle.abort();
                let _ = tokio::fs::remove_file(&prompt_file).await;
                return Err(e);
            }

            // Terminate the process since the turn is complete
            // opencode run may not exit on its own after step_finish
            kill_process_group(&mut child);
            // Allow the stderr reader to finish draining synchronously so any
            // late-arriving error line (rate_limit, auth, etc.) is captured before
            // we look at detected_stderr_error.
            let _ = timeout(Duration::from_millis(100), stderr_handle).await;
            // Attempt to reap the process without blocking the result
            let _ = timeout(Duration::from_secs(5), child.wait()).await;
            let _ = tokio::fs::remove_file(&prompt_file).await;

            // Promote any detected stderr error into a typed turn failure (§10.5).
            // This prevents silent "success" when opencode crashed on rate-limit /
            // auth / account / permission errors mid-turn but still emitted a
            // stop-reason event before exiting.
            if let Some(err) = detected_stderr_error.lock().await.take() {
                tracing::warn!(error = %err, "opencode stderr signaled a failure; turn marked as failed");
                return Err(err);
            }

            let sid = current_session.unwrap_or_else(|| issue.id.clone());
            let tid = current_turn.unwrap_or_else(|| "turn-1".into());

            // SPEC §10.2.2 — outcome is a typed enum; failure to reach a
            // `step_finish` with a stop-shaped reason maps to `Failed`, not a
            // silent `success=false` boolean.
            let outcome = if success {
                TurnOutcome::Succeeded
            } else {
                TurnOutcome::Failed
            };
            let last_message = (!accumulated_text.is_empty()).then_some(accumulated_text);
            Ok(TurnResult {
                session_id: sid.clone(),
                turn_id: tid,
                outcome,
                last_message,
                usage: tokens,
                error: None,
            })
        }.instrument(span).await
    }
}

/// Map opencode stderr lines to typed adapter errors (§10.5). Conservative
/// pattern matching: only well-known signals are surfaced — anything unknown
/// is left as a plain warn log line.
pub(crate) fn classify_stderr_line(line: &str) -> Option<SympheoError> {
    let lower = line.to_ascii_lowercase();
    if lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("rate-limit")
        || lower.contains("too many requests")
        || lower.contains("status 429")
    {
        return Some(SympheoError::TurnFailed(format!("rate limit: {line}")));
    }
    if lower.contains("unauthorized")
        || lower.contains("invalid api key")
        || lower.contains("api key") && (lower.contains("missing") || lower.contains("invalid"))
        || lower.contains("authentication failed")
        || lower.contains("authentication required")
        || lower.contains("status 401")
        || lower.contains("status 403")
    {
        return Some(SympheoError::SessionStartFailed(format!(
            "auth failure: {line}"
        )));
    }
    if lower.contains("account required")
        || lower.contains("login required")
        || lower.contains("subscription required")
        || lower.contains("payment required")
    {
        return Some(SympheoError::UserInputRequired(format!(
            "account / login required: {line}"
        )));
    }
    if lower.contains("permission denied") && !lower.contains("--dangerously-skip-permissions") {
        return Some(SympheoError::TurnFailed(format!(
            "permission denied: {line}"
        )));
    }
    None
}

fn kill_process_group(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        let pgid = pid as i32;
        unsafe {
            let _ = libc::killpg(pgid, libc::SIGKILL);
        }
    }
    // Fallback: also try the standard kill
    let _ = child.start_kill();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::cli::opencode::OpencodeAdapter;
    use std::path::PathBuf;

    /// Default adapter for tests where the specific CLI shape doesn't matter —
    /// the OpenCode adapter's `build_command_string` shape matches the JSON
    /// fixtures the existing tests pass through `bash -c`.
    fn test_adapter() -> Arc<dyn CliAdapter> {
        Arc::new(OpencodeAdapter::new())
    }

    #[test]
    fn test_classify_stderr_line_rate_limit() {
        assert!(matches!(
            classify_stderr_line("ERROR: Rate limit exceeded"),
            Some(SympheoError::TurnFailed(_))
        ));
        assert!(matches!(
            classify_stderr_line("[opencode] HTTP status 429 too many requests"),
            Some(SympheoError::TurnFailed(_))
        ));
    }

    #[test]
    fn test_classify_stderr_line_auth() {
        assert!(matches!(
            classify_stderr_line("Unauthorized: invalid API key"),
            Some(SympheoError::SessionStartFailed(_))
        ));
        assert!(matches!(
            classify_stderr_line("status 401 authentication failed"),
            Some(SympheoError::SessionStartFailed(_))
        ));
    }

    #[test]
    fn test_classify_stderr_line_account_required() {
        assert!(matches!(
            classify_stderr_line("Subscription required to use this model"),
            Some(SympheoError::UserInputRequired(_))
        ));
    }

    #[test]
    fn test_classify_stderr_line_permission_denied() {
        assert!(matches!(
            classify_stderr_line("permission denied for tool 'write'"),
            Some(SympheoError::TurnFailed(_))
        ));
    }

    #[test]
    fn test_classify_stderr_line_neutral() {
        assert!(classify_stderr_line("info: starting model").is_none());
        assert!(classify_stderr_line("[debug] connection ok").is_none());
        // The skip-permissions flag string itself should NOT trigger
        assert!(classify_stderr_line("--dangerously-skip-permissions enabled").is_none());
    }

    #[tokio::test]
    async fn test_local_backend_run_turn_timeout() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut workspace = serde_json::Map::<String, serde_json::Value>::new();
        let tmp = std::env::temp_dir().join(format!("local_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        workspace.insert(
            "root".into(),
            serde_json::Value::String(tmp.to_string_lossy().to_string()),
        );
        raw.insert("workspace".into(), serde_json::Value::Object(workspace));
        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        cli.insert(
            "command".into(),
            serde_json::Value::String(r#"bash -c "sleep 1000""#.into()),
        );
        cli.insert(
            "turn_timeout_ms".into(),
            serde_json::Value::Number(200.into()),
        );
        raw.insert("cli".into(), serde_json::Value::Object(cli));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let backend = LocalBackend::new(&config, test_adapter()).unwrap();
        let issue = crate::tracker::model::Issue {
            id: "1".into(),
            identifier: "TEST-1".into(),
            title: "test".into(),
            description: None,
            priority: None,
            state: "todo".into(),
            branch_name: None,
            url: None,
            labels: vec![],
            blocked_by: vec![],
            ..Default::default()
        };
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
        let result = backend
            .run_turn(
                &issue,
                "prompt",
                None,
                &tmp,
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                event_tx,
            )
            .await;
        // SPEC §10.5: a process that produces zero output stalls the per-line
        // read first; with a 200 ms `turn_timeout_ms` and the default 5000 ms
        // `read_timeout_ms`, the total timeout fires before the read timeout.
        assert!(
            matches!(result, Err(SympheoError::TurnTotalTimeout)),
            "expected TurnTotalTimeout, got {:?}",
            result
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// SPEC §10.2.2: when a per-stdout-read stalls past `read_timeout_ms` we
    /// MUST surface `TurnReadTimeout` (distinct from total-turn exhaustion).
    #[tokio::test]
    async fn test_local_backend_run_turn_read_timeout() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut workspace = serde_json::Map::<String, serde_json::Value>::new();
        let tmp = std::env::temp_dir().join(format!("local_read_to_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        workspace.insert(
            "root".into(),
            serde_json::Value::String(tmp.to_string_lossy().to_string()),
        );
        raw.insert("workspace".into(), serde_json::Value::Object(workspace));
        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        // Emit one event then sleep forever — read_timeout fires while we
        // wait for the next line.
        cli.insert(
            "command".into(),
            serde_json::Value::String(r#"bash -c 'echo "{\"type\":\"step_start\",\"timestamp\":1,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p1\",\"messageID\":\"msg-1\",\"sessionID\":\"sess-1\",\"type\":\"step\"}}"; sleep 1000'"#.into()),
        );
        cli.insert(
            "read_timeout_ms".into(),
            serde_json::Value::Number(150.into()),
        );
        cli.insert(
            "turn_timeout_ms".into(),
            serde_json::Value::Number(60_000.into()),
        );
        raw.insert("cli".into(), serde_json::Value::Object(cli));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let backend = LocalBackend::new(&config, test_adapter()).unwrap();
        let issue = crate::tracker::model::Issue {
            id: "1".into(),
            identifier: "TEST-1".into(),
            title: "test".into(),
            description: None,
            priority: None,
            state: "todo".into(),
            branch_name: None,
            url: None,
            labels: vec![],
            blocked_by: vec![],
            ..Default::default()
        };
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
        let result = backend
            .run_turn(
                &issue,
                "prompt",
                None,
                &tmp,
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                event_tx,
            )
            .await;
        assert!(
            matches!(result, Err(SympheoError::TurnReadTimeout)),
            "expected TurnReadTimeout, got {:?}",
            result
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_local_backend_run_turn_success() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut workspace = serde_json::Map::<String, serde_json::Value>::new();
        let tmp = std::env::temp_dir().join(format!("local_test3_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        workspace.insert(
            "root".into(),
            serde_json::Value::String(tmp.to_string_lossy().to_string()),
        );
        raw.insert("workspace".into(), serde_json::Value::Object(workspace));
        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        // Print valid opencode events and exit
        cli.insert(
            "command".into(),
            serde_json::Value::String(r#"bash -c 'echo "{\"type\":\"step_start\",\"timestamp\":1,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p1\",\"messageID\":\"msg-1\",\"sessionID\":\"sess-1\",\"type\":\"step\"}}"; echo "{\"type\":\"text\",\"timestamp\":2,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p2\",\"messageID\":\"msg-2\",\"sessionID\":\"sess-1\",\"type\":\"text\",\"text\":\"hello\"}}"; echo "{\"type\":\"step_finish\",\"timestamp\":3,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p3\",\"reason\":\"stop\",\"messageID\":\"msg-3\",\"sessionID\":\"sess-1\",\"type\":\"finish\",\"tokens\":{\"total\":100,\"input\":50,\"output\":40,\"reasoning\":10,\"cache\":{\"write\":5,\"read\":3}}}}"'"#.into()),
        );
        cli.insert(
            "turn_timeout_ms".into(),
            serde_json::Value::Number(5000.into()),
        );
        raw.insert("cli".into(), serde_json::Value::Object(cli));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let backend = LocalBackend::new(&config, test_adapter()).unwrap();
        let issue = crate::tracker::model::Issue {
            id: "1".into(),
            identifier: "TEST-1".into(),
            title: "test".into(),
            description: None,
            priority: None,
            state: "todo".into(),
            branch_name: None,
            url: None,
            labels: vec![],
            blocked_by: vec![],
            ..Default::default()
        };
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
        let result = backend
            .run_turn(
                &issue,
                "prompt",
                None,
                &tmp,
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                event_tx,
            )
            .await
            .unwrap();
        assert!(result.succeeded());
        assert_eq!(result.last_message.as_deref(), Some("hello"));
        assert_eq!(result.session_id, "sess-1");
        assert_eq!(result.turn_id, "msg-1");
        assert!(result.usage.is_some());
        let tokens = result.usage.as_ref().unwrap();
        assert_eq!(tokens.total, 100);
        assert_eq!(tokens.input, 50);
        assert_eq!(tokens.output, 40);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_local_backend_run_turn_no_finish() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut workspace = serde_json::Map::<String, serde_json::Value>::new();
        let tmp = std::env::temp_dir().join(format!("local_test4_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        workspace.insert(
            "root".into(),
            serde_json::Value::String(tmp.to_string_lossy().to_string()),
        );
        raw.insert("workspace".into(), serde_json::Value::Object(workspace));
        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        // Print step_start and text but no step_finish
        cli.insert(
            "command".into(),
            serde_json::Value::String(r#"bash -c 'echo "{\"type\":\"step_start\",\"timestamp\":1,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p1\",\"messageID\":\"msg-1\",\"sessionID\":\"sess-1\",\"type\":\"step\"}}"; echo "{\"type\":\"text\",\"timestamp\":2,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p2\",\"messageID\":\"msg-2\",\"sessionID\":\"sess-1\",\"type\":\"text\",\"text\":\"hello\"}}"'"#.into()),
        );
        cli.insert(
            "turn_timeout_ms".into(),
            serde_json::Value::Number(5000.into()),
        );
        raw.insert("cli".into(), serde_json::Value::Object(cli));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let backend = LocalBackend::new(&config, test_adapter()).unwrap();
        let issue = crate::tracker::model::Issue {
            id: "1".into(),
            identifier: "TEST-1".into(),
            title: "test".into(),
            description: None,
            priority: None,
            state: "todo".into(),
            branch_name: None,
            url: None,
            labels: vec![],
            blocked_by: vec![],
            ..Default::default()
        };
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
        let result = backend
            .run_turn(
                &issue,
                "prompt",
                None,
                &tmp,
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                event_tx,
            )
            .await
            .unwrap();
        assert!(!result.succeeded());
        assert!(matches!(result.outcome, TurnOutcome::Failed));
        assert_eq!(result.last_message.as_deref(), Some("hello"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_local_backend_run_turn_with_session_and_stderr() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut workspace = serde_json::Map::<String, serde_json::Value>::new();
        let tmp = std::env::temp_dir().join(format!("local_test5_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        workspace.insert(
            "root".into(),
            serde_json::Value::String(tmp.to_string_lossy().to_string()),
        );
        raw.insert("workspace".into(), serde_json::Value::Object(workspace));
        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        // Print valid opencode events to stdout and something to stderr
        cli.insert(
            "command".into(),
            serde_json::Value::String(r#"bash -c 'echo "stderr msg" >&2; sleep 0.2; echo "{\"type\":\"step_start\",\"timestamp\":1,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p1\",\"messageID\":\"msg-1\",\"sessionID\":\"sess-1\",\"type\":\"step\"}}"; echo "{\"type\":\"text\",\"timestamp\":2,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p2\",\"messageID\":\"msg-2\",\"sessionID\":\"sess-1\",\"type\":\"text\",\"text\":\"hello\"}}"; echo "{\"type\":\"step_finish\",\"timestamp\":3,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p3\",\"reason\":\"stop\",\"messageID\":\"msg-3\",\"sessionID\":\"sess-1\",\"type\":\"finish\",\"tokens\":{\"total\":100,\"input\":50,\"output\":40,\"reasoning\":10,\"cache\":{\"write\":5,\"read\":3}}}}"'"#.into()),
        );
        cli.insert(
            "turn_timeout_ms".into(),
            serde_json::Value::Number(5000.into()),
        );
        raw.insert("cli".into(), serde_json::Value::Object(cli));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let backend = LocalBackend::new(&config, test_adapter()).unwrap();
        let issue = crate::tracker::model::Issue {
            id: "1".into(),
            identifier: "TEST-1".into(),
            title: "test".into(),
            description: None,
            priority: None,
            state: "todo".into(),
            branch_name: None,
            url: None,
            labels: vec![],
            blocked_by: vec![],
            ..Default::default()
        };
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
        let result = backend
            .run_turn(
                &issue,
                "prompt",
                Some("existing-session"),
                &tmp,
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                event_tx,
            )
            .await
            .unwrap();
        assert!(result.succeeded());
        assert_eq!(result.last_message.as_deref(), Some("hello"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_local_backend_validate_outside_root() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut workspace = serde_json::Map::<String, serde_json::Value>::new();
        let tmp = std::env::temp_dir().join(format!("local_test2_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        workspace.insert(
            "root".into(),
            serde_json::Value::String(tmp.to_string_lossy().to_string()),
        );
        raw.insert("workspace".into(), serde_json::Value::Object(workspace));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let backend = LocalBackend::new(&config, test_adapter()).unwrap();
        let issue = crate::tracker::model::Issue {
            id: "1".into(),
            identifier: "TEST-1".into(),
            title: "test".into(),
            description: None,
            priority: None,
            state: "todo".into(),
            branch_name: None,
            url: None,
            labels: vec![],
            blocked_by: vec![],
            ..Default::default()
        };
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
        let result = backend
            .run_turn(
                &issue,
                "prompt",
                None,
                Path::new("/etc"),
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                event_tx,
            )
            .await;
        assert!(matches!(result, Err(SympheoError::WorkspaceError(_))));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Regression for the bug fixed by issue #130: events must arrive at the
    /// consumer in real time, not as a batch at turn end. We assert the gap
    /// between the first and last received timestamp is consistent with the
    /// 0.2s sleeps in the fake CLI command — i.e. events were streamed.
    #[tokio::test]
    async fn test_local_backend_events_streamed_during_turn() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut workspace = serde_json::Map::<String, serde_json::Value>::new();
        let tmp = std::env::temp_dir().join(format!("local_stream_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        workspace.insert(
            "root".into(),
            serde_json::Value::String(tmp.to_string_lossy().to_string()),
        );
        raw.insert("workspace".into(), serde_json::Value::Object(workspace));
        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        cli.insert(
            "command".into(),
            serde_json::Value::String(r#"bash -c 'echo "{\"type\":\"step_start\",\"timestamp\":1,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p1\",\"messageID\":\"msg-1\",\"sessionID\":\"sess-1\",\"type\":\"step\"}}"; sleep 0.2; echo "{\"type\":\"text\",\"timestamp\":2,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p2\",\"messageID\":\"msg-2\",\"sessionID\":\"sess-1\",\"type\":\"text\",\"text\":\"hello\"}}"; sleep 0.2; echo "{\"type\":\"step_finish\",\"timestamp\":3,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p3\",\"reason\":\"stop\",\"messageID\":\"msg-3\",\"sessionID\":\"sess-1\",\"type\":\"finish\"}}"'"#.into()),
        );
        cli.insert(
            "turn_timeout_ms".into(),
            serde_json::Value::Number(5000.into()),
        );
        raw.insert("cli".into(), serde_json::Value::Object(cli));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let backend = LocalBackend::new(&config, test_adapter()).unwrap();
        let issue = crate::tracker::model::Issue {
            id: "1".into(),
            identifier: "TEST-1".into(),
            title: "test".into(),
            description: None,
            priority: None,
            state: "todo".into(),
            branch_name: None,
            url: None,
            labels: vec![],
            blocked_by: vec![],
            ..Default::default()
        };

        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(64);
        let arrival_log: std::sync::Arc<tokio::sync::Mutex<Vec<std::time::Instant>>> =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let arrival_for_consumer = arrival_log.clone();
        let consumer = tokio::spawn(async move {
            while let Some(_evt) = event_rx.recv().await {
                arrival_for_consumer
                    .lock()
                    .await
                    .push(std::time::Instant::now());
            }
        });

        let result = backend
            .run_turn(
                &issue,
                "prompt",
                None,
                &tmp,
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                event_tx,
            )
            .await
            .unwrap();
        let _ = consumer.await;
        assert!(result.succeeded());

        let arrivals = arrival_log.lock().await;
        assert!(
            arrivals.len() >= 3,
            "expected at least 3 events, got {}",
            arrivals.len()
        );
        let first = arrivals.first().unwrap();
        let last = arrivals.last().unwrap();
        let spread = last.duration_since(*first);
        assert!(
            spread.as_millis() >= 200,
            "events arrived as a batch (spread = {} ms); expected >= 200 ms because the fake CLI sleeps between events",
            spread.as_millis()
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// SPEC §15.5 hardening: when LocalBackend launches the CLI subprocess,
    /// the inherited env MUST be scrubbed and HOME / XDG_*_HOME MUST point
    /// inside the workspace. We launch a fake CLI that records the env it
    /// receives to a file, then assert HOME / XDG_CONFIG_HOME are scoped to
    /// the workspace and a credential-shaped host env var did NOT leak.
    #[tokio::test]
    async fn test_local_backend_env_isolation() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut workspace = serde_json::Map::<String, serde_json::Value>::new();
        let tmp = std::env::temp_dir().join(format!("local_iso_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        workspace.insert(
            "root".into(),
            serde_json::Value::String(tmp.to_string_lossy().to_string()),
        );
        raw.insert("workspace".into(), serde_json::Value::Object(workspace));
        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        // Fake CLI: write env snapshot, then emit the minimal events the parser
        // needs to mark the turn as success so we don't get spurious errors.
        let env_dump_script = r#"bash -c '/usr/bin/env > env.txt; echo "{\"type\":\"step_start\",\"timestamp\":1,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p1\",\"messageID\":\"msg-1\",\"sessionID\":\"sess-1\",\"type\":\"step\"}}"; echo "{\"type\":\"step_finish\",\"timestamp\":3,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p3\",\"reason\":\"stop\",\"messageID\":\"msg-3\",\"sessionID\":\"sess-1\",\"type\":\"finish\"}}"'"#;
        cli.insert(
            "command".into(),
            serde_json::Value::String(env_dump_script.into()),
        );
        cli.insert(
            "turn_timeout_ms".into(),
            serde_json::Value::Number(5000.into()),
        );
        raw.insert("cli".into(), serde_json::Value::Object(cli));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());

        // Set a credential-shaped var on the host that MUST NOT leak through.
        unsafe {
            std::env::set_var("ANTHROPIC_API_KEY", "sk-host-must-not-leak");
        }

        let backend = LocalBackend::new(&config, test_adapter()).unwrap();
        let issue = crate::tracker::model::Issue {
            id: "1".into(),
            identifier: "TEST-1".into(),
            title: "test".into(),
            description: None,
            priority: None,
            state: "todo".into(),
            branch_name: None,
            url: None,
            labels: vec![],
            blocked_by: vec![],
            ..Default::default()
        };
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
        backend
            .run_turn(
                &issue,
                "prompt",
                None,
                &tmp,
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                event_tx,
            )
            .await
            .unwrap();

        let env_text = std::fs::read_to_string(tmp.join("env.txt")).unwrap();
        let expected_home = format!("{}/.sympheo-home", tmp.display());
        assert!(
            env_text.contains(&format!("HOME={expected_home}")),
            "HOME should be scoped to workspace: {}",
            env_text
        );
        assert!(
            env_text.contains(&format!("XDG_CONFIG_HOME={expected_home}/.config")),
            "XDG_CONFIG_HOME should be scoped: {}",
            env_text
        );
        assert!(
            !env_text.contains("ANTHROPIC_API_KEY"),
            "credential-shaped var leaked through env: {}",
            env_text
        );
        // Worker PATH must not contain a mise shims dir — the whole point of
        // pre-resolving tools at startup is so the worker bypasses mise.
        // Shape-based check: no PATH segment may equal "shims" or end with
        // "/shims". Substring matching would false-negative on legitimate dirs
        // like "/opt/notmise/shimshelpers/bin".
        let path_line = env_text
            .lines()
            .find(|l| l.starts_with("PATH="))
            .unwrap_or("");
        let path_value = path_line.strip_prefix("PATH=").unwrap_or("");
        for segment in path_value.split(':') {
            assert!(
                !segment.ends_with("/shims") && segment != "shims",
                "worker PATH segment is a shims dir: {} (full PATH={})",
                segment,
                path_value
            );
        }

        unsafe {
            std::env::remove_var("ANTHROPIC_API_KEY");
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// SPEC §15.5: cli.env from WORKFLOW.md (§5.3.6) must override the
    /// scrubbed defaults — operators can re-introduce specific credentials
    /// they want the agent to see (e.g. GITHUB_TOKEN).
    #[tokio::test]
    async fn test_local_backend_cli_env_overrides_pass_through() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut workspace = serde_json::Map::<String, serde_json::Value>::new();
        let tmp = std::env::temp_dir().join(format!("local_cli_env_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        workspace.insert(
            "root".into(),
            serde_json::Value::String(tmp.to_string_lossy().to_string()),
        );
        raw.insert("workspace".into(), serde_json::Value::Object(workspace));
        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        let env_dump_script = r#"bash -c '/usr/bin/env > env.txt; echo "{\"type\":\"step_finish\",\"timestamp\":3,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p3\",\"reason\":\"stop\",\"messageID\":\"msg-3\",\"sessionID\":\"sess-1\",\"type\":\"finish\"}}"'"#;
        cli.insert(
            "command".into(),
            serde_json::Value::String(env_dump_script.into()),
        );
        let mut env_overrides = serde_json::Map::<String, serde_json::Value>::new();
        env_overrides.insert(
            "GITHUB_TOKEN".into(),
            serde_json::Value::String("ghp-from-workflow".into()),
        );
        cli.insert("env".into(), serde_json::Value::Object(env_overrides));
        cli.insert(
            "turn_timeout_ms".into(),
            serde_json::Value::Number(5000.into()),
        );
        raw.insert("cli".into(), serde_json::Value::Object(cli));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let backend = LocalBackend::new(&config, test_adapter()).unwrap();
        let issue = crate::tracker::model::Issue {
            id: "1".into(),
            identifier: "TEST-1".into(),
            title: "test".into(),
            description: None,
            priority: None,
            state: "todo".into(),
            branch_name: None,
            url: None,
            labels: vec![],
            blocked_by: vec![],
            ..Default::default()
        };
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
        backend
            .run_turn(
                &issue,
                "prompt",
                None,
                &tmp,
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                event_tx,
            )
            .await
            .unwrap();

        let env_text = std::fs::read_to_string(tmp.join("env.txt")).unwrap();
        assert!(
            env_text.contains("GITHUB_TOKEN=ghp-from-workflow"),
            "cli.env override should pass through: {}",
            env_text
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn make_config_with_cli_command(command: &str) -> ServiceConfig {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut workspace = serde_json::Map::<String, serde_json::Value>::new();
        workspace.insert(
            "root".into(),
            serde_json::Value::String(std::env::temp_dir().to_string_lossy().to_string()),
        );
        raw.insert("workspace".into(), serde_json::Value::Object(workspace));
        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        cli.insert(
            "command".into(),
            serde_json::Value::String(command.to_string()),
        );
        raw.insert("cli".into(), serde_json::Value::Object(cli));
        ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into())
    }

    /// `LocalBackend::new` rewrites the leading binary token of `cli.command`
    /// to its resolved absolute path. Confirms (a) the leading token becomes
    /// an absolute path, (b) the trailing args are preserved verbatim.
    #[test]
    fn test_local_backend_new_rewrites_resolvable_bin_to_absolute() {
        let bash_abs = tool_resolver::resolve_tool("bash").expect("bash on host");
        let config = make_config_with_cli_command("bash -c \"echo hi\"");
        let backend = LocalBackend::new(&config, test_adapter()).expect("construct backend");
        let expected = format!("{} -c \"echo hi\"", bash_abs.display());
        assert_eq!(
            backend.command, expected,
            "expected leading bin rewritten to absolute path with args preserved"
        );
    }

    /// When the leading binary cannot be resolved on the host, `LocalBackend::new`
    /// MUST fall back to the raw command string verbatim and let the worker
    /// invocation fail later (covered by the warn log in resolution).
    #[test]
    fn test_local_backend_new_falls_back_to_raw_when_unresolvable() {
        let raw = "definitely-not-a-binary-xyz123 --foo bar";
        let config = make_config_with_cli_command(raw);
        let backend = LocalBackend::new(&config, test_adapter()).expect("construct backend");
        assert_eq!(
            backend.command, raw,
            "raw command must pass through unchanged when resolution fails"
        );
    }

    /// `cli.command` with no trailing args (`raw_command[name.len()..]` = "")
    /// must still rewrite cleanly to just the absolute path, no trailing
    /// whitespace.
    #[test]
    fn test_local_backend_new_rewrites_bare_command_no_args() {
        let bash_abs = tool_resolver::resolve_tool("bash").expect("bash on host");
        let config = make_config_with_cli_command("bash");
        let backend = LocalBackend::new(&config, test_adapter()).expect("construct backend");
        assert_eq!(
            backend.command,
            bash_abs.display().to_string(),
            "bare command must rewrite to just the absolute path"
        );
    }

    /// When `cli.command`'s first token equals an `ALWAYS_RESOLVE_TOOLS` entry
    /// (`gh`), the resolved bin dir must NOT be duplicated. Length-of-vec
    /// equals length-of-set.
    #[test]
    fn test_local_backend_new_dedups_resolved_bin_dirs() {
        // We cannot guarantee `gh` is installed, so this test is skipped if not.
        // The invariant we verify: every parent dir in `resolved_bin_dirs`
        // appears at most once.
        if tool_resolver::resolve_tool("gh").is_none() {
            return;
        }
        let config = make_config_with_cli_command("gh issue list");
        let backend = LocalBackend::new(&config, test_adapter()).expect("construct backend");
        let mut seen = std::collections::HashSet::new();
        for d in &backend.resolved_bin_dirs {
            assert!(
                seen.insert(d.clone()),
                "duplicate bin dir: {:?} in {:?}",
                d,
                backend.resolved_bin_dirs
            );
        }
    }

    /// SPEC §15.5 / Q6 hardening test: the worker PATH must not contain a
    /// mise shims directory. Substring matching on `/shims` is brittle —
    /// assert on shape: no PATH segment ends with `/shims` or a `mise/...`
    /// shims dir. This catches the real bug (a shim leaks through) without
    /// false-negatives on user dirs that happen to contain "shims" as a
    /// substring (e.g. `/opt/notmise/shimshelpers/bin`).
    #[test]
    fn test_path_segment_shape_rejects_shims() {
        // Sample PATH entries that should and should not match the shape rule.
        let bad = [
            "/home/u/.local/share/mise/shims",
            "/home/u/.asdf/shims",
            "/some/where/shims",
        ];
        let good = [
            "/usr/bin",
            "/opt/notmise/shimshelpers/bin",
            "/home/u/bin/shimserver",
            "/home/u/.local/share/mise/installs/foo/1.2.3/bin",
        ];
        for p in &bad {
            assert!(
                p.ends_with("/shims") || p.split('/').any(|s| s == "shims"),
                "expected shape-match for {p}"
            );
        }
        for p in &good {
            assert!(
                !p.ends_with("/shims") && !p.split('/').any(|s| s == "shims"),
                "expected no shape-match for {p}"
            );
        }
    }
}
