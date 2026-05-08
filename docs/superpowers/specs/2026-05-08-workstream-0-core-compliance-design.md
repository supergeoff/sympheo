# Workstream 0 — Core Compliance & SPEC Conformance

> **Date:** 2026-05-08  
> **Scope:** All tasks 0.1–0.8 + tests 0.9  
> **Approach:** Bottom-up (model → config → parser → orchestrator → tests)

---

## Context

The Sympheo orchestrator has functional gaps between its current implementation and the contract defined in `SPEC.md`. This workstream closes those gaps to improve reliability, observability, and spec conformance.

---

## Goals

1. Liquid template rendering fails on unknown variables (strict mode).
2. Token accounting uses deltas to prevent double-counting.
3. Rate-limit payloads are extracted from agent events and stored in state.
4. The continuation prompt is configurable via `WORKFLOW.md`.
5. The worker logs `AttemptStatus` transitions during its lifecycle.
6. Config defaults align with SPEC (with explicit deviations documented).
7. All 13 agent events are parseable; critical events update orchestrator state.
8. Tests cover dispatch sort, reconciliation, stall, backoff, strict Liquid, and token delta.

---

## Non-Goals

- Changing the tracker from GitHub to Linear (out of scope).
- Full `codex app-server` protocol conformance (Daytona workstream).
- Timer-based retry scheduling (task 0.6 skipped; polling retry is functional and tested).

---

## Architecture

### 1. Data Model Changes (`src/tracker/model.rs`)

#### `RunAttempt`
Add a constructor and a transition helper:

```rust
impl RunAttempt {
    pub fn new(
        issue_id: String,
        issue_identifier: String,
        attempt: u32,
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
            attempt = self.attempt,
            issue = %self.issue_identifier,
            ?status,
            "Attempt status transition"
        );
        self.status = status;
    }
}
```

#### `LiveSession`
Add fields to capture the latest event from the agent stream:

```rust
pub struct LiveSession {
    // ... existing fields ...
    pub last_event: Option<String>,
    pub last_message: Option<String>,
    pub last_event_at: Option<chrono::DateTime<chrono::Utc>>,
}
```

### 2. Config Changes (`src/config/typed.rs`)

#### `continuation_prompt()`
```rust
pub fn continuation_prompt(&self) -> String {
    self.agent()
        .and_then(|m| resolver::get_string(m, "continuation_prompt"))
        .unwrap_or_else(|| {
            "Continue working on the current task. Review the conversation history and proceed with the next step.".into()
        })
}
```

#### `codex_command()`
Change default from `"opencode run"` to `"codex app-server"`.

#### `tracker_endpoint()`
When `kind == "linear"`, default to `"https://api.linear.app/graphql"`. Keep GitHub default for `kind == "github"`.

> **Breaking change mitigation:** The repo's `WORKFLOW.md` must explicitly set `codex.command: opencode run` to preserve current behavior.

### 3. Agent Event Model (`src/agent/parser.rs`)

New enum `AgentEvent`:

```rust
#[derive(Debug, Clone)]
pub enum AgentEvent {
    SessionStarted { session_id: String, thread_id: String },
    TurnCompleted { session_id: String, turn_id: String, tokens: Option<TokenInfo> },
    TurnFailed { session_id: String, reason: String },
    TurnCancelled { session_id: String },
    TurnInputRequired { session_id: String },
    ApprovalAutoApproved { session_id: String, kind: String },
    Notification { session_id: String, message: String },
    RateLimit { payload: serde_json::Value },
    TokenUsage { input: u64, output: u64, total: u64 },
    StepStart { id: String, name: String },
    Text { content: String },
    StepFinish { id: String, result: String },
    Malformed { raw: String },
}
```

Parsing strategy:
- Each line of stdout is treated as a JSON object.
- Try to deserialize into `AgentEvent` using `serde_json`.
- On parse failure, emit `AgentEvent::Malformed { raw: line }`.
- Existing `OpencodeEvent` can be replaced by `AgentEvent` or kept as an internal alias during migration.

### 4. Streaming Contract (`agent/backend/mod.rs`, `agent/runner.rs`)

#### Trait `AgentBackend`
Change `run_turn` signature:

```rust
async fn run_turn(
    &self,
    issue: &Issue,
    prompt: &str,
    session_id: Option<&str>,
    workspace_path: &std::path::Path,
) -> Result<(TurnResult, tokio::sync::mpsc::Receiver<AgentEvent>), SympheoError>;
```

#### `AgentRunner`
Forward the receiver from the selected backend unchanged.

#### `LocalBackend`
- Spawn the agent process.
- Create an `mpsc::channel(128)`.
- Read stdout line-by-line in a spawned task; parse each line into `AgentEvent` and send on the channel.
- When the process exits, send a final `TurnCompleted` or `TurnFailed` event, then close the sender.
- Return the `TurnResult` (overall success/failure) alongside the receiver.

#### `DaytonaBackend`
Same pattern: parse the stdout returned by the Daytona execute API line-by-line, send events on the channel. (Full Daytona lifecycle improvements are Workstream 1; here we only adapt the existing execute path to the new streaming contract.)

### 5. Orchestrator Integration (`src/orchestrator/tick.rs`)

#### `run_worker()` lifecycle

```rust
let mut attempt = RunAttempt::new(
    issue.id.clone(),
    issue.identifier.clone(),
    attempt_number,
    workspace_path.clone(),
);

// Workspace preparation
attempt.transition(AttemptStatus::PreparingWorkspace);
// ... workspace manager calls ...

// Prompt building
attempt.transition(AttemptStatus::BuildingPrompt);
let prompt = if turn_number == 1 {
    build_prompt_strict(config, issue, Some(attempt_number), skill_instructions)?
} else {
    config.continuation_prompt()
};

// Agent launch
attempt.transition(AttemptStatus::LaunchingAgentProcess);
let (turn_result, mut event_rx) = runner.run_turn(...).await?;

// Streaming
attempt.transition(AttemptStatus::StreamingTurn);
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
                    sess.last_event = Some(format!("{:?}", event));
                    sess.last_message = Some(message.clone());
                    sess.last_event_at = Some(chrono::Utc::now());
                }
                _ => {}
            }
        }
    }
}

attempt.transition(AttemptStatus::Finishing);
```

#### `build_prompt_strict()`

```rust
fn build_prompt_strict(
    config: &ServiceConfig,
    issue: &Issue,
    attempt: Option<u32>,
    skill_instructions: Option<&str>,
) -> Result<String, SympheoError> {
    let template_str = config.prompt_template()?;
    let available_vars = ["issue", "attempt", "skill"];

    // Extract variables from template
    let re = regex::Regex::new(r"\{\{\s*([a-zA-Z_][a-zA-Z0-9_.]*)\s*\}\}")?;
    for cap in re.captures_iter(&template_str) {
        let var_path = cap.get(1).unwrap().as_str();
        let root = var_path.split('.').next().unwrap();
        if !available_vars.contains(&root) {
            return Err(SympheoError::TemplateRenderError(
                format!("Unknown variable: {}", root).into()
            ));
        }
    }

    let template = liquid::ParserBuilder::with_stdlib()
        .build()
        .map_err(|e| SympheoError::TemplateParseError(e.to_string()))?
        .parse(&template_str)
        .map_err(|e| SympheoError::TemplateParseError(e.to_string()))?;

    let mut globals = liquid::object!({
        "issue": issue,
        "attempt": attempt,
    });
    if let Some(skill) = skill_instructions {
        globals.insert("skill".into(), liquid::model::Value::Scalar(skill.into()));
    }

    template.render(&globals)
        .map_err(|e| SympheoError::TemplateRenderError(e.to_string()))
}
```

> Note: Liquid's `object!` macro serializes the `Issue` struct via serde. Nested fields like `issue.title` work because serde flattens the struct. The strictness check validates the **root variable name** only; this is sufficient because the SPEC requires "Unknown variables MUST fail rendering," and a misspelled root var (`{{ issu.title }}`) will be caught. Accessing a nonexistent field on an existing object (`{{ issue.nonexistent }}`) will render as empty in Liquid; if the SPEC later requires this to fail too, we can add a post-render regex check as a follow-up.

### 6. Tests (`tests/`)

New and updated test files:

| Test | File | What it verifies |
|---|---|---|
| Strict Liquid unknown root var | `tests/integration_test.rs` or `tests/liquid_strict_test.rs` | `{{ unknown }}` in template → `TemplateRenderError` |
| Strict Liquid unknown nested var | Same | `{{ issue.unknown_field }}` renders empty (documented behavior) |
| Token delta no double-count | `tests/integration_test.rs` | Two turns with same token totals → global total unchanged |
| Token delta accumulation | Same | Turn with 100 input, then 150 input → global input = 150 |
| Dispatch sort order | `tests/integration_test.rs` | Issues sorted by priority → created_at → identifier |
| Reconciliation terminal cleanup | `tests/integration_test.rs` | Terminal issue → worker stops, workspace removed |
| Stall detection | `tests/integration_test.rs` | Old `last_timestamp` → reconcile terminates entry |
| Backoff formula | `src/orchestrator/retry.rs` (existing) | Already covered; verify no regression |
| Agent event parsing | `tests/integration_test.rs` | `RateLimit`, `TurnFailed`, `TokenUsage` parse correctly |
| Config continuation prompt | `src/config/typed.rs` (existing) | `agent.continuation_prompt` parsed from YAML |

---

## Error Handling

- `TemplateRenderError` on strict Liquid failure.
- `AgentRunnerError` on backend streaming failure (channel closed unexpectedly).
- All event parsing errors are non-fatal: `Malformed` events are logged at `warn` level but do not stop the worker.

---

## Performance & Safety

- The `mpsc` channel buffer size is 128. If the orchestrator is slower than the event producer, the sender blocks backpressure. This is acceptable because the orchestrator's event handling is lightweight (state update only).
- Token arithmetic uses `saturating_sub` to prevent underflow.
- The `RunAttempt` struct is stack-allocated in `run_worker` and never cloned; it exists only for logging.

---

## Rollback Plan

If the streaming contract causes regressions in `LocalBackend`, the old `run_turn` signature can be temporarily restored by wrapping the new implementation:
```rust
async fn run_turn_legacy(...) -> Result<TurnResult, SympheoError> {
    let (result, mut rx) = self.run_turn(...).await?;
    while rx.recv().await.is_some() {} // drain
    Ok(result)
}
```

---

## Acceptance Criteria

- [ ] `cargo test` passes with ≥ 44 tests (existing 38 + 6 new).
- [ ] `cargo clippy` passes with no new warnings.
- [ ] Unknown root Liquid variables fail rendering.
- [ ] Token accounting uses deltas (identical reports on consecutive turns do not inflate totals).
- [ ] `RateLimit` events update `codex_rate_limits` in state.
- [ ] `WORKFLOW.md` can override `agent.continuation_prompt`.
- [ ] Worker logs show `PreparingWorkspace → BuildingPrompt → LaunchingAgentProcess → StreamingTurn → Finishing` transitions.
- [ ] The repo's `WORKFLOW.md` explicitly sets `codex.command: opencode run` to preserve current behavior.
