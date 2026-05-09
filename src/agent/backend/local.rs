use crate::agent::backend::AgentBackend;
use crate::agent::parser::{AgentEvent, TokenInfo, TurnResult, parse_event_line};
use crate::config::typed::ServiceConfig;
use crate::error::SympheoError;
use crate::tracker::model::Issue;
use crate::workspace::manager::WorkspaceManager;
use async_trait::async_trait;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::Sender;
use tokio::time::{Duration, timeout};
use tracing::Instrument;

pub struct LocalBackend {
    command: String,
    turn_timeout: Duration,
    workspace_manager: WorkspaceManager,
}

impl LocalBackend {
    pub fn new(config: &ServiceConfig) -> Result<Self, SympheoError> {
        Ok(Self {
            command: config.cli_command(),
            turn_timeout: Duration::from_millis(config.cli_turn_timeout_ms()),
            workspace_manager: WorkspaceManager::new(config)?,
        })
    }
}

#[async_trait]
impl AgentBackend for LocalBackend {
    async fn run_turn(
        &self,
        issue: &Issue,
        prompt: &str,
        session_id: Option<&str>,
        workspace_path: &Path,
        cancelled: Arc<AtomicBool>,
        event_tx: Sender<AgentEvent>,
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

            let sanitized = sanitize_prompt_for_opencode(prompt);
            tracing::debug!(prompt_len = sanitized.len(), "prompt length");

            // Write prompt to a temp file to avoid shell escaping and ARG_MAX issues
            let prompt_file = workspace_path.join(format!(".sympheo_prompt_{}.txt", issue.id));
            tokio::fs::write(&prompt_file, &sanitized).await
                .map_err(|e| SympheoError::AgentRunnerError(format!("failed to write prompt file: {e}")))?;

            let mut cmd = Command::new("bash");
            cmd.arg("-lc");

            let mut opencode_cmd = format!(
                r#"PROMPT=$(cat "{}"); {} "$PROMPT" --format json --dir "{}" --dangerously-skip-permissions"#,
                shell_escape(&prompt_file.to_string_lossy()),
                self.command,
                shell_escape(&workspace_path.to_string_lossy())
            );
            if let Some(sid) = session_id {
                opencode_cmd.push_str(&format!(" --session {}", shell_escape(sid)));
            }
            cmd.arg(&opencode_cmd);
            cmd.current_dir(workspace_path);
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::piped());

            // Run in a new process group so we can kill the entire tree reliably
            unsafe {
                cmd.pre_exec(|| {
                    libc::setpgid(0, 0);
                    Ok(())
                });
            }

            tracing::info!("launching opencode agent (local backend)");
            tracing::debug!(command = %opencode_cmd, "local backend command");

            let mut child = cmd
                .spawn()
                .map_err(|e| SympheoError::AgentRunnerError(format!("spawn failed: {e}")))?;

            // Track PID and start cancellation watchdog
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

            // Spawn stderr reader task to capture agent diagnostic output
            let stderr_span = tracing::info_span!(parent: span_clone, "opencode_stderr");
            let stderr_handle = tokio::spawn(async move {
                let stderr_reader = BufReader::new(stderr);
                let mut stderr_lines = stderr_reader.lines();
                while let Ok(Some(line)) = stderr_lines.next_line().await {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        tracing::warn!(target = "opencode::stderr", "{}", trimmed);
                    }
                }
            }.instrument(stderr_span));

            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            let mut current_session: Option<String> = None;
            let mut current_turn: Option<String> = None;
            let mut accumulated_text = String::new();
            let mut tokens: Option<TokenInfo> = None;
            let mut success = false;

            let read_result = timeout(self.turn_timeout, async {
                while let Ok(Some(line)) = lines.next_line().await {
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Some(event) = parse_event_line(&line) {
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
                        let _ = event_tx.send(event).await;
                    } else {
                        tracing::warn!(raw = %line, "failed to parse event line");
                    }
                }
            })
            .await;

            if read_result.is_err() {
                kill_process_group(&mut child);
                let _ = stderr_handle.abort();
                let _ = tokio::fs::remove_file(&prompt_file).await;
                return Err(SympheoError::AgentTurnTimeout);
            }

            // Terminate the process since the turn is complete
            // opencode run may not exit on its own after step_finish
            kill_process_group(&mut child);
            let _ = stderr_handle.abort();
            // Attempt to reap the process without blocking the result
            let _ = timeout(Duration::from_secs(5), child.wait()).await;
            let _ = tokio::fs::remove_file(&prompt_file).await;

            let sid = current_session.unwrap_or_else(|| issue.id.clone());
            let tid = current_turn.unwrap_or_else(|| "turn-1".into());

            Ok(TurnResult {
                session_id: sid.clone(),
                turn_id: tid,
                success,
                text: accumulated_text,
                tokens,
            })
        }.instrument(span).await
    }
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

fn shell_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`")
        .replace('\'', "\\'")
        .replace(';', "\\;")
        .replace('|', "\\|")
        .replace('&', "\\&")
        .replace('<', "\\<")
        .replace('>', "\\>")
        .replace('(', "\\(")
        .replace(')', "\\)")
        .replace('*', "\\*")
        .replace('?', "\\?")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('\n', "\\n")
}

fn sanitize_prompt_for_opencode(prompt: &str) -> String {
    let re = regex::Regex::new(r"(?m)^--[a-z0-9-]+$").unwrap();
    re.replace_all(prompt, |caps: &regex::Captures| format!("`{}`", &caps[0]))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_shell_escape_backslash() {
        assert_eq!(shell_escape("a\\b"), "a\\\\b");
    }

    #[test]
    fn test_shell_escape_quote() {
        assert_eq!(shell_escape("say \"hi\""), "say \\\"hi\\\"");
    }

    #[test]
    fn test_shell_escape_dollar() {
        assert_eq!(shell_escape("$HOME"), "\\$HOME");
    }

    #[test]
    fn test_shell_escape_backtick() {
        assert_eq!(shell_escape("`cmd`"), "\\`cmd\\`");
    }

    #[test]
    fn test_shell_escape_combined() {
        assert_eq!(shell_escape("\\\"$`"), "\\\\\\\"\\$\\`");
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
        let backend = LocalBackend::new(&config).unwrap();
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
            matches!(result, Err(SympheoError::AgentTurnTimeout)),
            "expected AgentTurnTimeout, got {:?}",
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
        let backend = LocalBackend::new(&config).unwrap();
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
        assert!(result.success);
        assert_eq!(result.text, "hello");
        assert_eq!(result.session_id, "sess-1");
        assert_eq!(result.turn_id, "msg-1");
        assert!(result.tokens.is_some());
        let tokens = result.tokens.unwrap();
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
        let backend = LocalBackend::new(&config).unwrap();
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
        assert!(!result.success);
        assert_eq!(result.text, "hello");
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
        let backend = LocalBackend::new(&config).unwrap();
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
        assert!(result.success);
        assert_eq!(result.text, "hello");
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
        let backend = LocalBackend::new(&config).unwrap();
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
        let backend = LocalBackend::new(&config).unwrap();
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
        assert!(result.success);

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
}
