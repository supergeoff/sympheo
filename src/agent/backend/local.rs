use async_trait::async_trait;
use crate::agent::backend::AgentBackend;
use crate::agent::parser::{parse_line, OpencodeEvent, TokenInfo, TurnResult};
use crate::config::typed::ServiceConfig;
use crate::error::SymphonyError;
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
    pub fn new(config: &ServiceConfig) -> Result<Self, SymphonyError> {
        Ok(Self {
            command: config.codex_command(),
            turn_timeout: Duration::from_millis(config.codex_turn_timeout_ms()),
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
    ) -> Result<TurnResult, SymphonyError> {
        self.workspace_manager
            .validate_inside_root(workspace_path)?;

        let mut cmd = Command::new("bash");
        cmd.arg("-lc");

        let mut opencode_cmd = format!(
            r#"{} "{}" --format json --dir {}"#,
            self.command,
            shell_escape(prompt),
            shell_escape(&workspace_path.to_string_lossy())
        );
        if let Some(sid) = session_id {
            opencode_cmd.push_str(&format!(" --session {}", shell_escape(sid)));
        }
        cmd.arg(&opencode_cmd);
        cmd.current_dir(workspace_path);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        tracing::info!(
            issue_id = %issue.id,
            issue_identifier = %issue.identifier,
            "launching opencode agent (local backend)"
        );

        let mut child = cmd
            .spawn()
            .map_err(|e| SymphonyError::AgentRunnerError(format!("spawn failed: {e}")))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SymphonyError::AgentRunnerError("missing stdout".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| SymphonyError::AgentRunnerError("missing stderr".into()))?;

        // Spawn stderr reader task to capture agent diagnostic output
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
                match parse_line(&line) {
                    Some(OpencodeEvent::StepStart { session_id, part, .. }) => {
                        current_session = Some(session_id);
                        current_turn = Some(part.message_id.clone());
                        tracing::debug!(session = %part.session_id, message = %part.message_id, "step_start");
                    }
                    Some(OpencodeEvent::Text { part, .. }) => {
                        accumulated_text.push_str(&part.text);
                    }
                    Some(OpencodeEvent::StepFinish { part, .. }) => {
                        tokens = part.tokens.clone();
                        success = part.reason == "stop" || part.reason == "tool-calls";
                        tracing::info!(
                            reason = %part.reason,
                            success,
                            "step_finish"
                        );
                        break;
                    }
                    _ => {}
                }
            }
        })
        .await;

        if read_result.is_err() {
            let _ = child.kill().await;
            let _ = stderr_handle.abort();
            return Err(SymphonyError::AgentTurnTimeout);
        }

        // Terminate the process since the turn is complete
        // opencode run may not exit on its own after step_finish
        let _ = child.kill().await;
        let _ = stderr_handle.abort();
        // Attempt to reap the process without blocking the result
        let _ = timeout(Duration::from_secs(5), child.wait()).await;

        let sid = current_session.unwrap_or_else(|| issue.id.clone());
        let tid = current_turn.unwrap_or_else(|| "turn-1".into());

        Ok(TurnResult {
            session_id: sid.clone(),
            turn_id: tid,
            success,
            text: accumulated_text,
            tokens,
        })
    }
}

fn shell_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`")
}
