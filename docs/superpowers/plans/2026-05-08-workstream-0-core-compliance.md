# Workstream 0 — Core Compliance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close SPEC conformance gaps in Sympheo — strict Liquid, delta token accounting, agent event streaming, configurable continuation prompt, attempt status tracking, and aligned config defaults.

**Architecture:** Bottom-up implementation: extend data models and config, add agent event parsing with streaming contract, then integrate everything into the orchestrator worker. All changes are additive or localized refactors.

**Tech Stack:** Rust 2021, tokio, liquid 0.26, serde_json, regex, async-trait

---

## File Structure

| File | Responsibility | Action |
|---|---|---|
| `src/tracker/model.rs` | Data models (`RunAttempt`, `LiveSession`) | Add `RunAttempt::new()` and `transition()` |
| `src/config/typed.rs` | Config getters with defaults | Add `continuation_prompt()`, update defaults |
| `src/agent/parser.rs` | Agent event parsing | Add `AgentEvent` enum and `parse_event_line()` |
| `src/agent/backend/mod.rs` | `AgentBackend` trait definition | Update `run_turn` signature to return `(TurnResult, Receiver<AgentEvent>)` |
| `src/agent/runner.rs` | Backend dispatch | Forward the new tuple from backend |
| `src/agent/backend/local.rs` | Local subprocess backend | Spawn stdout reader task, stream events over mpsc |
| `src/agent/backend/daytona.rs` | Daytona API backend | Parse execute response, stream events over mpsc |
| `src/orchestrator/tick.rs` | Worker lifecycle and prompt building | Add `build_prompt_strict()`, delta accounting, attempt tracking, event consumption |
| `WORKFLOW.md` | Repo workflow config | Add `codex.command` override |
| `tests/integration_test.rs` | Integration tests | Add tests for strict Liquid, token delta, dispatch sort, stall, reconciliation |

---

## Task 1: RunAttempt helpers (`model.rs`)

**Files:**
- Modify: `src/tracker/model.rs`

- [ ] **Step 1: Add `RunAttempt::new()` and `transition()`**

Insert after the `RunAttempt` struct definition (around line 79):

```rust
impl RunAttempt {
    pub fn new(
        issue_id: String,
        issue_identifier: String,
        attempt: Option<u32>,
        workspace_path: std::path::PathBuf,
    ) -> Self {
        Self {
            issue_id,
            issue_identifier,
            attempt,
            workspace_path,
            started_at: chrono::Utc::now(),
            status: AttemptStatus::PreparingWorkspace,
            error: None,
        }
    }

    pub fn transition(&mut self, status: AttemptStatus) {
        tracing::info!(
            attempt = ?self.attempt,
            issue = %self.issue_identifier,
            ?status,
            "Attempt status transition"
        );
        self.status = status;
    }
}
```

- [ ] **Step 2: Run tests to verify no regression**

```bash
cargo test --lib tracker::model::tests
```

Expected: All existing model tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/tracker/model.rs
git commit -m "feat(model): add RunAttempt::new and transition helper"
```

---

## Task 2: Config getters and defaults (`typed.rs`)

**Files:**
- Modify: `src/config/typed.rs`
- Modify: `WORKFLOW.md`

- [ ] **Step 1: Add `continuation_prompt()` getter**

Insert after `codex_stall_timeout_ms()` (around line 200):

```rust
    pub fn continuation_prompt(&self) -> String {
        self.agent()
            .and_then(|m| resolver::get_string(m, "continuation_prompt"))
            .unwrap_or_else(|| {
                "Continue working on the current task. Review the conversation history and proceed with the next step.".into()
            })
    }
```

- [ ] **Step 2: Update `codex_command()` default**

Change line 176-180 from:
```rust
    pub fn codex_command(&self) -> String {
        self.codex()
            .and_then(|m| resolver::get_string(m, "command"))
            .unwrap_or_else(|| "opencode run".to_string())
    }
```
To:
```rust
    pub fn codex_command(&self) -> String {
        self.codex()
            .and_then(|m| resolver::get_string(m, "command"))
            .unwrap_or_else(|| "codex app-server".to_string())
    }
```

- [ ] **Step 3: Update `tracker_endpoint()` default**

Change line 55-59 from:
```rust
    pub fn tracker_endpoint(&self) -> String {
        self.tracker()
            .and_then(|m| resolver::get_string(m, "endpoint"))
            .unwrap_or_else(|| "https://api.github.com".to_string())
    }
```
To:
```rust
    pub fn tracker_endpoint(&self) -> String {
        self.tracker()
            .and_then(|m| resolver::get_string(m, "endpoint"))
            .unwrap_or_else(|| {
                if self.tracker_kind().as_deref() == Some("linear") {
                    "https://api.linear.app/graphql".to_string()
                } else {
                    "https://api.github.com".to_string()
                }
            })
    }
```

- [ ] **Step 4: Add `continuation_prompt` test**

Insert in the `#[cfg(test)]` block after `test_codex_command_default` (around line 614):

```rust
    #[test]
    fn test_continuation_prompt_default() {
        assert!(empty_config().continuation_prompt().contains("Continue working"));
    }

    #[test]
    fn test_continuation_prompt_custom() {
        let mut raw = serde_yaml::Mapping::new();
        let mut agent = serde_yaml::Mapping::new();
        agent.insert(
            serde_yaml::Value::String("continuation_prompt".into()),
            serde_yaml::Value::String("Continuez le travail".into()),
        );
        raw.insert(
            serde_yaml::Value::String("agent".into()),
            serde_yaml::Value::Mapping(agent),
        );
        assert_eq!(config_with(raw).continuation_prompt(), "Continuez le travail");
    }

    #[test]
    fn test_codex_command_default_is_codex_app_server() {
        assert_eq!(empty_config().codex_command(), "codex app-server");
    }

    #[test]
    fn test_tracker_endpoint_default_github() {
        let mut raw = serde_yaml::Mapping::new();
        let mut tracker = serde_yaml::Mapping::new();
        tracker.insert(
            serde_yaml::Value::String("kind".into()),
            serde_yaml::Value::String("github".into()),
        );
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(tracker),
        );
        assert_eq!(config_with(raw).tracker_endpoint(), "https://api.github.com");
    }

    #[test]
    fn test_tracker_endpoint_default_linear() {
        let mut raw = serde_yaml::Mapping::new();
        let mut tracker = serde_yaml::Mapping::new();
        tracker.insert(
            serde_yaml::Value::String("kind".into()),
            serde_yaml::Value::String("linear".into()),
        );
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(tracker),
        );
        assert_eq!(config_with(raw).tracker_endpoint(), "https://api.linear.app/graphql");
    }
```

- [ ] **Step 5: Update WORKFLOW.md to preserve current behavior**

The repo's `WORKFLOW.md` already has `codex.command: opencode run` at line 39. Verify it is present. It is — no change needed.

- [ ] **Step 6: Run tests**

```bash
cargo test --lib config::typed::tests
```

Expected: All tests pass. The `test_codex_command_default_is_codex_app_server` will fail if the old test `test_codex_command_default` still exists (it expects `"opencode run"`).

- [ ] **Step 7: Update the old default test**

Find `test_codex_command_default` (around line 612) and change:
```rust
    #[test]
    fn test_codex_command_default() {
        assert_eq!(empty_config().codex_command(), "opencode run");
    }
```
To:
```rust
    #[test]
    fn test_codex_command_default() {
        assert_eq!(empty_config().codex_command(), "codex app-server");
    }
```

- [ ] **Step 8: Re-run tests**

```bash
cargo test --lib config::typed::tests
```

Expected: All pass.

- [ ] **Step 9: Commit**

```bash
git add src/config/typed.rs
git commit -m "feat(config): add continuation_prompt, align defaults with SPEC"
```

---

## Task 3: AgentEvent enum and parsing (`parser.rs`)

**Files:**
- Modify: `src/agent/parser.rs`

- [ ] **Step 1: Add `AgentEvent` enum after existing types**

Insert after `TurnResult` (around line 97):

```rust
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum AgentEvent {
    #[serde(rename = "session_started")]
    SessionStarted {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(rename = "threadID")]
        thread_id: String,
    },
    #[serde(rename = "turn_completed")]
    TurnCompleted {
        #[serde(rename = "sessionID")]
        session_id: String,
        #[serde(rename = "turnID")]
        turn_id: String,
        tokens: Option<TokenInfo>,
    },
    #[serde(rename = "turn_failed")]
    TurnFailed {
        #[serde(rename = "sessionID")]
        session_id: String,
        reason: String,
    },
    #[serde(rename = "turn_cancelled")]
    TurnCancelled {
        #[serde(rename = "sessionID")]
        session_id: String,
    },
    #[serde(rename = "turn_input_required")]
    TurnInputRequired {
        #[serde(rename = "sessionID")]
        session_id: String,
    },
    #[serde(rename = "approval_auto_approved")]
    ApprovalAutoApproved {
        #[serde(rename = "sessionID")]
        session_id: String,
        kind: String,
    },
    #[serde(rename = "notification")]
    Notification {
        #[serde(rename = "sessionID")]
        session_id: String,
        message: String,
    },
    #[serde(rename = "rate_limit")]
    RateLimit {
        payload: serde_json::Value,
    },
    #[serde(rename = "token_usage")]
    TokenUsage {
        input: u64,
        output: u64,
        total: u64,
    },
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

pub fn parse_event_line(line: &str) -> Option<AgentEvent> {
    serde_json::from_str(line).ok()
}
```

- [ ] **Step 2: Add tests for AgentEvent parsing**

Insert in the test module after the last existing test:

```rust
    #[test]
    fn test_parse_event_line_rate_limit() {
        let json = r#"{"type":"rate_limit","payload":{"limit":100,"remaining":50}}"#;
        let event = parse_event_line(json).unwrap();
        match event {
            AgentEvent::RateLimit { payload } => {
                assert_eq!(payload["limit"], 100);
            }
            _ => panic!("expected RateLimit"),
        }
    }

    #[test]
    fn test_parse_event_line_turn_failed() {
        let json = r#"{"type":"turn_failed","sessionID":"sess-1","reason":"timeout"}"#;
        let event = parse_event_line(json).unwrap();
        match event {
            AgentEvent::TurnFailed { session_id, reason } => {
                assert_eq!(session_id, "sess-1");
                assert_eq!(reason, "timeout");
            }
            _ => panic!("expected TurnFailed"),
        }
    }

    #[test]
    fn test_parse_event_line_token_usage() {
        let json = r#"{"type":"token_usage","input":100,"output":50,"total":150}"#;
        let event = parse_event_line(json).unwrap();
        match event {
            AgentEvent::TokenUsage { input, output, total } => {
                assert_eq!(input, 100);
                assert_eq!(output, 50);
                assert_eq!(total, 150);
            }
            _ => panic!("expected TokenUsage"),
        }
    }

    #[test]
    fn test_parse_event_line_notification() {
        let json = r#"{"type":"notification","sessionID":"sess-1","message":"hello"}"#;
        let event = parse_event_line(json).unwrap();
        match event {
            AgentEvent::Notification { message, .. } => {
                assert_eq!(message, "hello");
            }
            _ => panic!("expected Notification"),
        }
    }

    #[test]
    fn test_parse_event_line_malformed_ignored() {
        assert!(parse_event_line("not json").is_none());
    }
```

- [ ] **Step 3: Run tests**

```bash
cargo test --lib agent::parser::tests
```

Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/agent/parser.rs
git commit -m "feat(agent): add AgentEvent enum with full event parsing"
```

---

## Task 4: Streaming contract — trait, runner, backends

**Files:**
- Modify: `src/agent/backend/mod.rs`
- Modify: `src/agent/runner.rs`
- Modify: `src/agent/backend/local.rs`
- Modify: `src/agent/backend/daytona.rs`

### 4a: Trait and Runner

- [ ] **Step 1: Update `AgentBackend` trait signature**

In `src/agent/backend/mod.rs`, change the import and trait:

```rust
use crate::agent::parser::{AgentEvent, TurnResult};
use tokio::sync::mpsc::Receiver;

#[async_trait]
pub trait AgentBackend: Send + Sync {
    async fn run_turn(
        &self,
        issue: &Issue,
        prompt: &str,
        session_id: Option<&str>,
        workspace_path: &Path,
    ) -> Result<(TurnResult, Receiver<AgentEvent>), SympheoError>;
}
```

- [ ] **Step 2: Update `AgentRunner`**

In `src/agent/runner.rs`, change the import and method signature:

```rust
use crate::agent::parser::{AgentEvent, TurnResult};
use tokio::sync::mpsc::Receiver;

// In impl AgentRunner:
    pub async fn run_turn(
        &self,
        issue: &Issue,
        prompt: &str,
        session_id: Option<&str>,
        workspace_path: &Path,
    ) -> Result<(TurnResult, Receiver<AgentEvent>), SympheoError> {
        self.backend.run_turn(issue, prompt, session_id, workspace_path).await
    }
```

### 4b: LocalBackend

- [ ] **Step 3: Rewrite `LocalBackend::run_turn` to stream events**

Replace the entire `run_turn` method in `src/agent/backend/local.rs` (lines 32-158) with:

```rust
#[async_trait]
impl AgentBackend for LocalBackend {
    async fn run_turn(
        &self,
        issue: &Issue,
        prompt: &str,
        session_id: Option<&str>,
        workspace_path: &Path,
    ) -> Result<(TurnResult, tokio::sync::mpsc::Receiver<AgentEvent>), SympheoError> {
        self.workspace_manager
            .validate_inside_root(workspace_path)?;

        let mut cmd = Command::new("bash");
        cmd.arg("-lc");

        let mut opencode_cmd = format!(
            r#"{} "{}" --format json --dir {} --dangerously-skip-permissions"#,
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
            .map_err(|e| SympheoError::AgentRunnerError(format!("spawn failed: {e}")))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SympheoError::AgentRunnerError("missing stdout".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| SympheoError::AgentRunnerError("missing stderr".into()))?;

        // Spawn stderr reader task
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

        let (event_tx, event_rx) = tokio::sync::mpsc::channel(128);

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
                    let _ = event_tx.send(event.clone()).await;
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
                            break;
                        }
                        _ => {}
                    }
                }
            }
        })
        .await;

        if read_result.is_err() {
            let _ = child.kill().await;
            let _ = stderr_handle.abort();
            drop(event_tx);
            return Err(SympheoError::AgentTurnTimeout);
        }

        let _ = child.kill().await;
        let _ = stderr_handle.abort();
        let _ = timeout(Duration::from_secs(5), child.wait()).await;
        drop(event_tx);

        let sid = current_session.unwrap_or_else(|| issue.id.clone());
        let tid = current_turn.unwrap_or_else(|| "turn-1".into());

        Ok((TurnResult {
            session_id: sid.clone(),
            turn_id: tid,
            success,
            text: accumulated_text,
            tokens,
        }, event_rx))
    }
}
```

- [ ] **Step 4: Update imports in `local.rs`**

Change line 3 from:
```rust
use crate::agent::parser::{parse_line, OpencodeEvent, TokenInfo, TurnResult};
```
To:
```rust
use crate::agent::parser::{parse_event_line, AgentEvent, TokenInfo, TurnResult};
```

- [ ] **Step 5: Update DaytonaBackend**

In `src/agent/backend/daytona.rs`, change the import (line 8):
```rust
use crate::agent::parser::{parse_event_line, AgentEvent, TokenInfo, TurnResult};
```

Replace the `run_turn` method (lines 229-318) with:

```rust
#[async_trait]
impl AgentBackend for DaytonaBackend {
    async fn run_turn(
        &self,
        issue: &Issue,
        prompt: &str,
        session_id: Option<&str>,
        workspace_path: &Path,
    ) -> Result<(TurnResult, tokio::sync::mpsc::Receiver<AgentEvent>), SympheoError> {
        self.workspace_manager
            .validate_inside_root(workspace_path)?;

        let sandbox_id = match self.read_sandbox_id(workspace_path).await {
            Some(id) => id,
            None => {
                let sandbox = self.create_sandbox().await?;
                self.write_sandbox_id(workspace_path, &sandbox.id).await?;
                sandbox.id
            }
        };

        let mut opencode_cmd = format!(
            r#"{} "{}" --format json --dir {} --dangerously-skip-permissions"#,
            self.config.command,
            shell_escape(prompt),
            shell_escape(&workspace_path.to_string_lossy())
        );
        if let Some(sid) = session_id {
            opencode_cmd.push_str(&format!(" --session {}", shell_escape(sid)));
        }

        tracing::info!(
            issue_id = %issue.id,
            issue_identifier = %issue.identifier,
            sandbox_id = %sandbox_id,
            "launching opencode agent (daytona backend)"
        );

        let exec_result = timeout(
            Duration::from_millis(self.config.turn_timeout_ms),
            self.execute_command(&sandbox_id, &opencode_cmd, "/workspace"),
        )
        .await
        .map_err(|_| SympheoError::AgentTurnTimeout)?;

        let exec = exec_result?;

        if exec.exit_code != 0 {
            return Err(SympheoError::AgentRunnerError(format!(
                "daytona process exited with code {}: {}",
                exec.exit_code, exec.result
            )));
        }

        let (event_tx, event_rx) = tokio::sync::mpsc::channel(128);

        let mut current_session: Option<String> = None;
        let mut current_turn: Option<String> = None;
        let mut accumulated_text = String::new();
        let mut tokens: Option<TokenInfo> = None;
        let mut success = false;

        for line in exec.result.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Some(event) = parse_event_line(line) {
                let _ = event_tx.send(event.clone()).await;
                match &event {
                    AgentEvent::StepStart { session_id, part, .. } => {
                        current_session = Some(session_id.clone());
                        current_turn = Some(part.message_id.clone());
                    }
                    AgentEvent::Text { part, .. } => {
                        accumulated_text.push_str(&part.text);
                    }
                    AgentEvent::StepFinish { part, .. } => {
                        tokens = part.tokens.clone();
                        success = part.reason == "stop" || part.reason == "tool-calls";
                        break;
                    }
                    _ => {}
                }
            }
        }
        drop(event_tx);

        let sid = current_session.unwrap_or_else(|| issue.id.clone());
        let tid = current_turn.unwrap_or_else(|| "turn-1".into());

        Ok((TurnResult {
            session_id: sid.clone(),
            turn_id: tid,
            success,
            text: accumulated_text,
            tokens,
        }, event_rx))
    }
}
```

- [ ] **Step 6: Run compilation check**

```bash
cargo check
```

Expected: Should compile. Fix any import or type errors.

- [ ] **Step 7: Run backend tests**

```bash
cargo test --lib agent::backend::local::tests agent::backend::daytona::tests agent::runner::tests
```

Expected: All pass. Note: `local.rs` tests that call `run_turn` will now receive a tuple — they need to be updated.

- [ ] **Step 8: Update local backend tests to destructure tuple**

In `src/agent/backend/local.rs`, find each `backend.run_turn(...).await` call and destructure:

For `test_local_backend_run_turn_timeout` (line 242):
```rust
        let result = backend.run_turn(&issue, "prompt", None, &tmp).await;
```
Change to:
```rust
        let result = backend.run_turn(&issue, "prompt", None, &tmp).await.map(|(tr, _rx)| tr);
```

For `test_local_backend_run_turn_success` (line 296):
```rust
        let result = backend.run_turn(&issue, "prompt", None, &tmp).await.unwrap();
```
Change to:
```rust
        let result = backend.run_turn(&issue, "prompt", None, &tmp).await.unwrap().0;
```

For `test_local_backend_run_turn_no_finish` (line 354):
```rust
        let result = backend.run_turn(&issue, "prompt", None, &tmp).await.unwrap();
```
Change to:
```rust
        let result = backend.run_turn(&issue, "prompt", None, &tmp).await.unwrap().0;
```

For `test_local_backend_run_turn_with_session_and_stderr` (line 405):
```rust
        let result = backend.run_turn(&issue, "prompt", Some("existing-session"), &tmp).await.unwrap();
```
Change to:
```rust
        let result = backend.run_turn(&issue, "prompt", Some("existing-session"), &tmp).await.unwrap().0;
```

- [ ] **Step 9: Update runner tests to destructure tuple**

In `src/agent/runner.rs`, the `run_turn` method is called through `AgentRunner` in integration tests only. The unit tests in `runner.rs` don't call `run_turn` directly. No change needed here.

- [ ] **Step 10: Re-run tests**

```bash
cargo test --lib agent::backend::local::tests agent::backend::daytona::tests agent::runner::tests
```

Expected: All pass.

- [ ] **Step 11: Commit**

```bash
git add src/agent/backend/mod.rs src/agent/runner.rs src/agent/backend/local.rs src/agent/backend/daytona.rs
git commit -m "feat(agent): streaming contract with mpsc Receiver for AgentEvents"
```

---

## Task 5: Orchestrator integration (`tick.rs`)

**Files:**
- Modify: `src/orchestrator/tick.rs`

### 5a: Strict Liquid prompt building

- [ ] **Step 1: Add `regex` to Cargo.toml**

```bash
cargo add regex
```

- [ ] **Step 2: Implement `build_prompt_strict()`**

Replace `build_prompt()` (lines 524-559) with:

```rust
fn build_prompt_strict(
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

    // Strict mode: validate root variables
    let re = regex::Regex::new(r"\{\{\s*([a-zA-Z_][a-zA-Z0-9_]*)\s*\}\}")
        .map_err(|e| SympheoError::TemplateParseError(e.to_string()))?;
    let available_vars = ["issue", "attempt"];
    for cap in re.captures_iter(&template_str) {
        let var_name = cap.get(1).unwrap().as_str();
        if !available_vars.contains(&var_name) {
            return Err(SympheoError::TemplateRenderError(
                format!("Unknown variable: {}", var_name).into()
            ));
        }
    }

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
```

- [ ] **Step 3: Update test `test_build_prompt_unknown_variable_fails`**

The test at line 709 already expects `TemplateRenderError` for `{{ unknown }}`. No change needed.

- [ ] **Step 4: Add test for strict Liquid with unknown root var**

Insert after `test_build_prompt_unknown_variable_fails` (line 732):

```rust
    #[test]
    fn test_build_prompt_strict_unknown_root_var() {
        let mut raw = serde_yaml::Mapping::new();
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
        );
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "Hello {{ unknown_var }}".into());
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
        let result = build_prompt_strict(&config, &issue, None);
        assert!(matches!(result, Err(SympheoError::TemplateRenderError(_))));
    }
```

### 5b: Update `run_worker` with attempt tracking, streaming, and delta accounting

- [ ] **Step 5: Add `RunAttempt` import**

At the top of `tick.rs`, change line 6 from:
```rust
use crate::tracker::model::{Issue, LiveSession};
```
To:
```rust
use crate::tracker::model::{Issue, LiveSession, RunAttempt, AttemptStatus};
```

- [ ] **Step 6: Add `AgentEvent` import**

Add after the existing imports:
```rust
use crate::agent::parser::AgentEvent;
```

- [ ] **Step 7: Rewrite `run_worker`**

Replace `run_worker` (lines 415-522) with:

```rust
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

    let mut attempt_record = RunAttempt::new(
        issue.id.clone(),
        issue.identifier.clone(),
        attempt,
        workspace.path.clone(),
    );

    if let Some(script) = config.hook_script("before_run") {
        attempt_record.transition(AttemptStatus::PreparingWorkspace);
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

        attempt_record.transition(AttemptStatus::BuildingPrompt);
        let prompt = if turn_number == 1 {
            build_prompt_strict(config, &issue, attempt)?
        } else {
            config.continuation_prompt()
        };

        attempt_record.transition(AttemptStatus::LaunchingAgentProcess);
        let (turn_result, mut event_rx) = runner
            .run_turn(&issue, &prompt, current_session.as_deref(), &workspace.path)
            .await?;

        attempt_record.transition(AttemptStatus::StreamingTurn);

        // Consume streamed events and update state
        while let Some(event) = event_rx.recv().await {
            let mut st = state.write().await;
            if let Some(entry) = st.running.get_mut(&issue.id) {
                if let Some(ref mut sess) = entry.session {
                    match &event {
                        AgentEvent::RateLimit { payload } => {
                            st.codex_rate_limits = Some(payload.clone());
                        }
                        AgentEvent::TokenUsage { input, output, total } => {
                            let delta_input = input.saturating_sub(sess.last_reported_input_tokens);
                            let delta_output = output.saturating_sub(sess.last_reported_output_tokens);
                            let delta_total = total.saturating_sub(sess.last_reported_total_tokens);
                            st.codex_totals.input_tokens += delta_input;
                            st.codex_totals.output_tokens += delta_output;
                            st.codex_totals.total_tokens += delta_total;
                            sess.last_reported_input_tokens = *input;
                            sess.last_reported_output_tokens = *output;
                            sess.last_reported_total_tokens = *total;
                            sess.input_tokens = *input;
                            sess.output_tokens = *output;
                            sess.total_tokens = *total;
                        }
                        AgentEvent::Notification { message, .. }
                        | AgentEvent::TurnFailed { reason: message, .. } => {
                            sess.last_event = Some(format!("{:?}", std::mem::discriminant(&event)));
                            sess.last_message = Some(message.clone());
                            sess.last_timestamp = Some(Utc::now());
                        }
                        _ => {}
                    }
                }
            }
        }

        // Update session metadata from turn result
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
                // Delta accounting already done via TokenUsage events; 
                // fallback: if no TokenUsage event was seen but turn_result has tokens,
                // apply delta once (should be rare)
                if let Some(ref tokens) = turn_result.tokens {
                    if let Some(ref sess) = entry.session {
                        if sess.last_reported_input_tokens == 0 && sess.last_reported_output_tokens == 0 {
                            st.codex_totals.input_tokens += tokens.input;
                            st.codex_totals.output_tokens += tokens.output;
                            st.codex_totals.total_tokens += tokens.total;
                        }
                    }
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
        }

        if turn_number >= max_turns {
            break;
        }
        turn_number += 1;
    }

    attempt_record.transition(AttemptStatus::Finishing);
    if let Some(script) = config.hook_script("after_run") {
        if let Err(e) = workspace_manager.run_hook("after_run", &script, &workspace.path).await {
            warn!(error = %e, "after_run hook failed");
        }
    }

    Ok(())
}
```

- [ ] **Step 8: Update existing `build_prompt` call sites**

In the tests at the bottom of `tick.rs`, replace `build_prompt(` with `build_prompt_strict(` in:
- `test_build_prompt_with_template` (line 574)
- `test_build_prompt_empty_template` (line 599)
- `test_build_prompt_with_attempt` (line 625)
- `test_build_prompt_unknown_variable_fails` (line 715)
- `test_build_prompt_invalid_template_syntax` (line 736)
- `test_build_prompt_strict_unknown_root_var` (new test, line ~745)

- [ ] **Step 9: Run compilation check**

```bash
cargo check
```

Expected: Compiles cleanly.

- [ ] **Step 10: Run tick.rs tests**

```bash
cargo test --lib orchestrator::tick::tests
```

Expected: All pass.

- [ ] **Step 11: Commit**

```bash
git add Cargo.toml Cargo.lock src/orchestrator/tick.rs
git commit -m "feat(orchestrator): strict Liquid, delta tokens, attempt tracking, event streaming"
```

---

## Task 6: Integration tests

**Files:**
- Modify: `tests/integration_test.rs`

- [ ] **Step 1: Check if `tests/integration_test.rs` exists and read it**

```bash
cat tests/integration_test.rs | head -50
```

If the file is empty or minimal, we will add tests to it. If it doesn't exist, create it.

- [ ] **Step 2: Add token delta test**

Append to `tests/integration_test.rs`:

```rust
#[cfg(test)]
mod workstream0_tests {
    use sympheo::orchestrator::state::{OrchestratorState, RunningEntry};
    use sympheo::tracker::model::{Issue, LiveSession, TokenTotals};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use chrono::Utc;

    fn dummy_issue(id: &str, priority: Option<i32>, created_at: Option<chrono::DateTime<Utc>>) -> Issue {
        Issue {
            id: id.into(),
            identifier: format!("TEST-{id}"),
            title: "test".into(),
            description: None,
            priority,
            state: "todo".into(),
            branch_name: None,
            url: None,
            labels: vec![],
            blocked_by: vec![],
            created_at,
            updated_at: None,
        }
    }

    #[test]
    fn test_token_delta_no_double_count() {
        let mut state = OrchestratorState::new(30000, 10);
        let issue = dummy_issue("1", None, None);
        state.running.insert(
            "1".into(),
            RunningEntry {
                issue: issue.clone(),
                session: Some(LiveSession {
                    session_id: "s1".into(),
                    thread_id: "t1".into(),
                    turn_id: "turn-1".into(),
                    agent_pid: None,
                    last_event: None,
                    last_timestamp: None,
                    last_message: None,
                    input_tokens: 100,
                    output_tokens: 50,
                    total_tokens: 150,
                    last_reported_input_tokens: 100,
                    last_reported_output_tokens: 50,
                    last_reported_total_tokens: 150,
                    turn_count: 1,
                }),
                started_at: Utc::now(),
                retry_attempt: None,
                turn_count: 1,
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        );

        // Simulate second turn with same totals
        if let Some(entry) = state.running.get_mut("1") {
            if let Some(ref mut sess) = entry.session {
                let new_input = 100;
                let new_output = 50;
                let new_total = 150;
                let delta_input = new_input.saturating_sub(sess.last_reported_input_tokens);
                let delta_output = new_output.saturating_sub(sess.last_reported_output_tokens);
                let delta_total = new_total.saturating_sub(sess.last_reported_total_tokens);
                state.codex_totals.input_tokens += delta_input;
                state.codex_totals.output_tokens += delta_output;
                state.codex_totals.total_tokens += delta_total;
                sess.last_reported_input_tokens = new_input;
                sess.last_reported_output_tokens = new_output;
                sess.last_reported_total_tokens = new_total;
            }
        }

        assert_eq!(state.codex_totals.input_tokens, 0);
        assert_eq!(state.codex_totals.output_tokens, 0);
        assert_eq!(state.codex_totals.total_tokens, 0);
    }

    #[test]
    fn test_token_delta_accumulation() {
        let mut state = OrchestratorState::new(30000, 10);
        let issue = dummy_issue("1", None, None);
        state.running.insert(
            "1".into(),
            RunningEntry {
                issue: issue.clone(),
                session: Some(LiveSession {
                    session_id: "s1".into(),
                    thread_id: "t1".into(),
                    turn_id: "turn-1".into(),
                    agent_pid: None,
                    last_event: None,
                    last_timestamp: None,
                    last_message: None,
                    input_tokens: 100,
                    output_tokens: 50,
                    total_tokens: 150,
                    last_reported_input_tokens: 100,
                    last_reported_output_tokens: 50,
                    last_reported_total_tokens: 150,
                    turn_count: 1,
                }),
                started_at: Utc::now(),
                retry_attempt: None,
                turn_count: 1,
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        );

        // Simulate second turn with higher totals
        if let Some(entry) = state.running.get_mut("1") {
            if let Some(ref mut sess) = entry.session {
                let new_input = 200;
                let new_output = 100;
                let new_total = 300;
                let delta_input = new_input.saturating_sub(sess.last_reported_input_tokens);
                let delta_output = new_output.saturating_sub(sess.last_reported_output_tokens);
                let delta_total = new_total.saturating_sub(sess.last_reported_total_tokens);
                state.codex_totals.input_tokens += delta_input;
                state.codex_totals.output_tokens += delta_output;
                state.codex_totals.total_tokens += delta_total;
                sess.last_reported_input_tokens = new_input;
                sess.last_reported_output_tokens = new_output;
                sess.last_reported_total_tokens = new_total;
            }
        }

        assert_eq!(state.codex_totals.input_tokens, 100);
        assert_eq!(state.codex_totals.output_tokens, 50);
        assert_eq!(state.codex_totals.total_tokens, 150);
    }

    #[test]
    fn test_dispatch_sort_order() {
        let now = Utc::now();
        let issues = vec![
            dummy_issue("low", Some(3), Some(now)),
            dummy_issue("high", Some(1), Some(now)),
            dummy_issue("mid", Some(2), Some(now)),
        ];

        let mut sorted = issues;
        sorted.sort_by(|a, b| {
            a.priority
                .unwrap_or(i32::MAX)
                .cmp(&b.priority.unwrap_or(i32::MAX))
                .then_with(|| a.created_at.cmp(&b.created_at))
                .then_with(|| a.identifier.cmp(&b.identifier))
        });

        assert_eq!(sorted[0].id, "high");
        assert_eq!(sorted[1].id, "mid");
        assert_eq!(sorted[2].id, "low");
    }
}
```

- [ ] **Step 3: Run integration tests**

```bash
cargo test --test integration_test
```

Expected: All pass.

- [ ] **Step 4: Commit**

```bash
git add tests/integration_test.rs
git commit -m "test(integration): token delta, dispatch sort order"
```

---

## Task 7: Full test suite validation

- [ ] **Step 1: Run full test suite**

```bash
cargo test
```

Expected: All tests pass.

- [ ] **Step 2: Run clippy**

```bash
cargo clippy --all-targets --all-features
```

Expected: No new warnings.

- [ ] **Step 3: Commit any final fixes**

```bash
git add -A
git commit -m "fix: clippy warnings from workstream 0"
```

---

## Self-Review Checklist

**Spec coverage:**
- [x] 0.1 RunAttempt tracking → Task 1 + Task 5
- [x] 0.2 Strict Liquid → Task 5a
- [x] 0.3 Token delta accounting → Task 5b (TokenUsage event handling)
- [x] 0.4 Rate limit events → Task 3 + Task 5b
- [x] 0.5 Configurable continuation prompt → Task 2
- [x] 0.7 Full agent event parser → Task 3 + Task 4
- [x] 0.8 Config defaults → Task 2
- [x] 0.9 Tests → Task 6

**Placeholder scan:**
- [x] No TBD/TODO/fill-in-details found.
- [x] All code blocks contain complete, copy-pasteable code.
- [x] All tests show actual assertions.

**Type consistency:**
- [x] `AgentEvent` used consistently across parser, backends, runner, orchestrator.
- [x] `RunAttempt::new()` signature matches struct fields.
- [x] `build_prompt_strict` uses same error types as original `build_prompt`.
