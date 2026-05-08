use async_trait::async_trait;
use crate::agent::backend::AgentBackend;
use crate::agent::parser::{parse_event_line, AgentEvent, TokenInfo, TurnResult};
use crate::config::typed::ServiceConfig;
use crate::error::SympheoError;
use crate::tracker::model::Issue;
use crate::workspace::manager::WorkspaceManager;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

pub struct LocalBackend {
    command: String,
    turn_timeout: Duration,
    workspace_manager: WorkspaceManager,
}

impl LocalBackend {
    pub fn new(config: &ServiceConfig) -> Result<Self, SympheoError> {
        Ok(Self {
            command: config.codex_command(),
            turn_timeout: Duration::from_millis(config.codex_turn_timeout_ms()),
            workspace_manager: WorkspaceManager::new(config)?,
        })
    }
}

#[async_trait]
impl AgentBackend for LocalBackend {
    /// Execute a single agent turn using the local backend.
    ///
    /// The agent process is spawned with both stdout and stderr piped.
    /// stdout is parsed for JSON agent events; stderr is consumed by a
    /// background task and logged at `WARN` level so diagnostic output
    /// is never lost. The stderr task is aborted when the turn finishes
    /// or times out, preventing leaked handles.
    async fn run_turn(
        &self,
        issue: &Issue,
        prompt: &str,
        session_id: Option<&str>,
        workspace_path: &Path,
    ) -> Result<(TurnResult, tokio::sync::mpsc::Receiver<AgentEvent>), SympheoError> {
        self.workspace_manager
            .validate_inside_root(workspace_path)?;

        // Write prompt to a temp file to avoid shell escaping and ARG_MAX issues
        let prompt_file = workspace_path.join(format!(".sympheo_prompt_{}.txt", issue.id));
        tokio::fs::write(&prompt_file, prompt).await
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

        tracing::info!(
            issue_id = %issue.id,
            issue_identifier = %issue.identifier,
            "launching opencode agent (local backend)"
        );
        tracing::debug!(command = %opencode_cmd, "local backend command");

        let mut child = cmd
            .spawn()
            .map_err(|e| SympheoError::AgentRunnerError(format!("spawn failed: {e}")))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SympheoError::AgentRunnerError("missing stdout".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| SympheoError::AgentRunnerError("missing stderr".into()))?;

        // Spawn an async reader task that consumes the agent's stderr stream.
        // Each non-empty line is logged at WARN level with the target
        // `opencode::stderr` and tagged with the issue ID so operators can
        // correlate diagnostic output with the ticket it belongs to.
        //
        // The task is aborted when the turn completes or times out so the
        // stderr pipe is always drained and never leaks.
        let issue_id_for_stderr = issue.id.clone();
        let stderr_handle = tokio::spawn(async move {
            let stderr_reader = BufReader::new(stderr);
            let mut stderr_lines = stderr_reader.lines();
            while let Ok(Some(line)) = stderr_lines.next_line().await {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    tracing::warn!(
                        issue_id = %issue_id_for_stderr,
                        target = "opencode::stderr",
                        "{}",
                        trimmed
                    );
                }
            }
        });

        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        let (event_tx, event_rx) = tokio::sync::mpsc::channel(100);

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
                                "step_finish"
                            );
                        }
                        _ => {}
                    }
                    let _ = event_tx.send(event).await;
                }
            }
        })
        .await;

        if read_result.is_err() {
            drop(event_tx);
            kill_process_group(&mut child);
            // Abort the stderr reader so the task does not outlive the turn.
            let _ = stderr_handle.abort();
            let _ = tokio::fs::remove_file(&prompt_file).await;
            return Err(SympheoError::AgentTurnTimeout);
        }

        // Terminate the process since the turn is complete.
        // opencode run may not exit on its own after step_finish.
        kill_process_group(&mut child);
        // Abort the stderr reader to ensure the task is reaped and no
        // handles are leaked before we return.
        let _ = stderr_handle.abort();
        // Attempt to reap the process without blocking the result
        let _ = timeout(Duration::from_secs(5), child.wait()).await;
        let _ = tokio::fs::remove_file(&prompt_file).await;

        let sid = current_session.unwrap_or_else(|| issue.id.clone());
        let tid = current_turn.unwrap_or_else(|| "turn-1".into());

        Ok((
            TurnResult {
                session_id: sid.clone(),
                turn_id: tid,
                success,
                text: accumulated_text,
                tokens,
            },
            event_rx,
        ))
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
        let mut raw = serde_yaml::Mapping::new();
        let mut workspace = serde_yaml::Mapping::new();
        let tmp = std::env::temp_dir().join(format!("local_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        workspace.insert(
            serde_yaml::Value::String("root".into()),
            serde_yaml::Value::String(tmp.to_string_lossy().to_string()),
        );
        raw.insert(
            serde_yaml::Value::String("workspace".into()),
            serde_yaml::Value::Mapping(workspace),
        );
        let mut codex = serde_yaml::Mapping::new();
        codex.insert(
            serde_yaml::Value::String("command".into()),
            serde_yaml::Value::String(r#"bash -c "sleep 1000""#.into()),
        );
        codex.insert(
            serde_yaml::Value::String("turn_timeout_ms".into()),
            serde_yaml::Value::Number(200.into()),
        );
        raw.insert(
            serde_yaml::Value::String("codex".into()),
            serde_yaml::Value::Mapping(codex),
        );
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
            created_at: None,
            updated_at: None,
        };
        let result = backend.run_turn(&issue, "prompt", None, &tmp).await.map(|(tr, _rx)| tr);
        assert!(
            matches!(result, Err(SympheoError::AgentTurnTimeout)),
            "expected AgentTurnTimeout, got {:?}",
            result
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_local_backend_run_turn_success() {
        let mut raw = serde_yaml::Mapping::new();
        let mut workspace = serde_yaml::Mapping::new();
        let tmp = std::env::temp_dir().join(format!("local_test3_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        workspace.insert(
            serde_yaml::Value::String("root".into()),
            serde_yaml::Value::String(tmp.to_string_lossy().to_string()),
        );
        raw.insert(
            serde_yaml::Value::String("workspace".into()),
            serde_yaml::Value::Mapping(workspace),
        );
        let mut codex = serde_yaml::Mapping::new();
        // Print valid opencode events and exit
        codex.insert(
            serde_yaml::Value::String("command".into()),
            serde_yaml::Value::String(r#"bash -c 'echo "{\"type\":\"step_start\",\"timestamp\":1,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p1\",\"messageID\":\"msg-1\",\"sessionID\":\"sess-1\",\"type\":\"step\"}}"; echo "{\"type\":\"text\",\"timestamp\":2,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p2\",\"messageID\":\"msg-2\",\"sessionID\":\"sess-1\",\"type\":\"text\",\"text\":\"hello\"}}"; echo "{\"type\":\"step_finish\",\"timestamp\":3,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p3\",\"reason\":\"stop\",\"messageID\":\"msg-3\",\"sessionID\":\"sess-1\",\"type\":\"finish\",\"tokens\":{\"total\":100,\"input\":50,\"output\":40,\"reasoning\":10,\"cache\":{\"write\":5,\"read\":3}}}}"'"#.into()),
        );
        codex.insert(
            serde_yaml::Value::String("turn_timeout_ms".into()),
            serde_yaml::Value::Number(5000.into()),
        );
        raw.insert(
            serde_yaml::Value::String("codex".into()),
            serde_yaml::Value::Mapping(codex),
        );
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
            created_at: None,
            updated_at: None,
        };
        let result = backend.run_turn(&issue, "prompt", None, &tmp).await.unwrap().0;
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
        let mut raw = serde_yaml::Mapping::new();
        let mut workspace = serde_yaml::Mapping::new();
        let tmp = std::env::temp_dir().join(format!("local_test4_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        workspace.insert(
            serde_yaml::Value::String("root".into()),
            serde_yaml::Value::String(tmp.to_string_lossy().to_string()),
        );
        raw.insert(
            serde_yaml::Value::String("workspace".into()),
            serde_yaml::Value::Mapping(workspace),
        );
        let mut codex = serde_yaml::Mapping::new();
        // Print step_start and text but no step_finish
        codex.insert(
            serde_yaml::Value::String("command".into()),
            serde_yaml::Value::String(r#"bash -c 'echo "{\"type\":\"step_start\",\"timestamp\":1,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p1\",\"messageID\":\"msg-1\",\"sessionID\":\"sess-1\",\"type\":\"step\"}}"; echo "{\"type\":\"text\",\"timestamp\":2,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p2\",\"messageID\":\"msg-2\",\"sessionID\":\"sess-1\",\"type\":\"text\",\"text\":\"hello\"}}"'"#.into()),
        );
        codex.insert(
            serde_yaml::Value::String("turn_timeout_ms".into()),
            serde_yaml::Value::Number(5000.into()),
        );
        raw.insert(
            serde_yaml::Value::String("codex".into()),
            serde_yaml::Value::Mapping(codex),
        );
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
            created_at: None,
            updated_at: None,
        };
        let result = backend.run_turn(&issue, "prompt", None, &tmp).await.unwrap().0;
        assert!(!result.success);
        assert_eq!(result.text, "hello");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_local_backend_run_turn_with_session_and_stderr() {
        let mut raw = serde_yaml::Mapping::new();
        let mut workspace = serde_yaml::Mapping::new();
        let tmp = std::env::temp_dir().join(format!("local_test5_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        workspace.insert(
            serde_yaml::Value::String("root".into()),
            serde_yaml::Value::String(tmp.to_string_lossy().to_string()),
        );
        raw.insert(
            serde_yaml::Value::String("workspace".into()),
            serde_yaml::Value::Mapping(workspace),
        );
        let mut codex = serde_yaml::Mapping::new();
        // Print valid opencode events to stdout and something to stderr
        codex.insert(
            serde_yaml::Value::String("command".into()),
            serde_yaml::Value::String(r#"bash -c 'echo "stderr msg" >&2; sleep 0.2; echo "{\"type\":\"step_start\",\"timestamp\":1,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p1\",\"messageID\":\"msg-1\",\"sessionID\":\"sess-1\",\"type\":\"step\"}}"; echo "{\"type\":\"text\",\"timestamp\":2,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p2\",\"messageID\":\"msg-2\",\"sessionID\":\"sess-1\",\"type\":\"text\",\"text\":\"hello\"}}"; echo "{\"type\":\"step_finish\",\"timestamp\":3,\"sessionID\":\"sess-1\",\"part\":{\"id\":\"p3\",\"reason\":\"stop\",\"messageID\":\"msg-3\",\"sessionID\":\"sess-1\",\"type\":\"finish\",\"tokens\":{\"total\":100,\"input\":50,\"output\":40,\"reasoning\":10,\"cache\":{\"write\":5,\"read\":3}}}}"'"#.into()),
        );
        codex.insert(
            serde_yaml::Value::String("turn_timeout_ms".into()),
            serde_yaml::Value::Number(5000.into()),
        );
        raw.insert(
            serde_yaml::Value::String("codex".into()),
            serde_yaml::Value::Mapping(codex),
        );
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
            created_at: None,
            updated_at: None,
        };
        let result = backend.run_turn(&issue, "prompt", Some("existing-session"), &tmp).await.unwrap().0;
        assert!(result.success);
        assert_eq!(result.text, "hello");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_local_backend_validate_outside_root() {
        let mut raw = serde_yaml::Mapping::new();
        let mut workspace = serde_yaml::Mapping::new();
        let tmp = std::env::temp_dir().join(format!("local_test2_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        workspace.insert(
            serde_yaml::Value::String("root".into()),
            serde_yaml::Value::String(tmp.to_string_lossy().to_string()),
        );
        raw.insert(
            serde_yaml::Value::String("workspace".into()),
            serde_yaml::Value::Mapping(workspace),
        );
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
            created_at: None,
            updated_at: None,
        };
        let result = backend.run_turn(&issue, "prompt", None, Path::new("/etc")).await;
        assert!(matches!(result, Err(SympheoError::WorkspaceError(_))));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
