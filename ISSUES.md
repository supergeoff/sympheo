# Sympheo — Issue Tracker & Technical Roadmap

> **Status:** Draft  
> **Author:** Tech Lead / Architect Review  
> **Date:** 2026-05-08  
> **Scope:** Core workflow resilience, OpenCode adapter hardening, GitHub API-native tracker, Git adapter decoupling

---

## Table of Contents

1. [Workstream 1: Workflow State Machine Hardening](#workstream-1-workflow-state-machine-hardening)
2. [Workstream 2: OpenCode Adapter — Resilience, Observability & Robustness](#workstream-2-opencode-adapter--resilience-observability--robustness)
3. [Workstream 3: GitHub Tracker — API-Native CRUD & Lifecycle Management](#workstream-3-github-tracker--api-native-crud--lifecycle-management)
4. [Workstream 4: Git Adapter — Decoupled Local SCM Operations](#workstream-4-git-adapter--decoupled-local-scm-operations)
5. [Cross-Cutting Concerns](#cross-cutting-concerns)
6. [Appendix A: Event Schema (OpenCode)](#appendix-a-event-schema-opencode)
7. [Appendix B: GitHub GraphQL Mutations Reference](#appendix-b-github-graphql-mutations-reference)

---

## Workstream 1: Workflow State Machine Hardening

### ISSUE-001: Define Explicit Skill for `todo` State

**Context & Problem:**
The current `WORKFLOW.md` maps skills to `spec`, `in progress`, `review`, `test`, `doc`. The `todo` state has **no mapped skill**. The base prompt template unconditionally instructs the agent to *"analyze the issue, implement the necessary changes, and ensure tests pass"*. This creates a fundamental contradiction: the agent receives a build instruction while sitting in the `todo` column, causing it to attempt full implementation without ever producing a specification or moving the ticket forward.

**Objective:**
Introduce a dedicated `todo` skill that acts as a **fast lane gatekeeper**: verify the ticket is well-formed, ensure prerequisites are documented, and immediately transition the ticket to `spec`.

**Technical Design:**

1. **New skill file:** `skills/todo/SKILL.md`
2. **Prompt injection strategy:** The skill prepends a strict system directive to the base template:
   - "You are a Workflow Gatekeeper. Your ONLY job is to verify this ticket and move it to the `Spec` column."
   - "Do NOT write code. Do NOT run tests. Do NOT modify source files."
   - "Check that the issue has: a clear title, an actionable description, and acceptance criteria."
   - "If the ticket is valid, move it to `Spec` using the GitHub API."
   - "If the ticket is unclear or missing information, append a comment explaining what is needed and STOP."
3. **Config update:** `WORKFLOW.md` skills mapping must include:
   ```yaml
   skills:
     mapping:
       todo: ./skills/todo/SKILL.md
       spec: ./skills/spec/SKILL.md
       # ... existing mappings
   ```
4. **Base prompt refactor:** Remove the unconditional *"implement the necessary changes"* from the base template. Replace with a state-conditional instruction:
   ```liquid
   {% case issue.state %}
   {% when "todo" %}
   Your task is to verify and advance this ticket to the Spec stage.
   {% when "spec" %}
   Your task is to produce a complete Low-Level Design (LLD).
   {% when "in progress" %}
   Your task is to implement the LLD with TDD discipline.
   # ... etc
   {% endcase %}
   ```

**Acceptance Criteria:**
- [x] `skills/todo/SKILL.md` exists and is loadable by `skills::loader`
- [x] `WORKFLOW.md` default template uses `{% case %}` or equivalent for state-conditional instructions
- [x] An agent dispatched for a `todo` ticket completes within **60 seconds** (fast lane)
- [x] The ticket is moved to `spec` via GitHub API (not `gh` CLI) after verification
- [x] If the ticket is malformed, a comment is posted and the ticket stays in `todo`

**Files Impacted:**
- `skills/todo/SKILL.md` (new)
- `WORKFLOW.md`
- `src/skills/mapper.rs` (verify lowercase key `"todo"` resolves correctly)

**Implementation Notes:**
- The `todo` skill must be short and deterministic. Avoid verbose markdown that could confuse the agent.
- Consider adding a `max_turns: 1` override for `todo` in the orchestrator config (future enhancement).

---

### ISSUE-002: Orchestrator State Transition Guardrails

**Context & Problem:**
The orchestrator relies entirely on the agent to move tickets via external commands (`gh` or API calls). If the agent fails to move the ticket, the orchestrator has no recovery strategy. It schedules a continuation retry (1s delay) with the **same state**, leading to infinite loops where the agent repeatedly does the same work.

**Objective:**
Add guardrails that detect stagnation and force progression or escalation.

**Technical Design:**

1. **Stagnation detection:**
   - Track `last_state_change_at` per issue in `RunningEntry`.
   - After each successful turn, if `issue.state == previous_state`, increment a `stagnation_counter`.
   - If `stagnation_counter >= max_stagnation_threshold` (default: 3), force a state transition or move to a `stuck` terminal state.

2. **Auto-advance fallback (optional, configurable):**
   - Add `workflow.auto_advance: bool` to `WORKFLOW.md`.
   - If `true` and the agent reports success but the ticket hasn't moved after N turns, the orchestrator itself calls the tracker to move the ticket to the next column.
   - If `false`, post a warning comment on the issue and stop tracking it.

3. **Max turns per state:**
   - Add `agent.max_turns_per_state: HashMap<String, u32>`.
   - Example: `todo: 1`, `spec: 3`, `in progress: 10`.
   - When `turn_number > max_for_state`, break the loop and trigger cleanup.

**Acceptance Criteria:**
- [x] `RunningEntry` tracks `stagnation_counter` and `last_state_change_at`
- [x] After 3 stagnant successful turns, orchestrator logs a clear warning and removes the issue from `claimed`
- [x] Configurable `auto_advance` flag exists in `ServiceConfig`
- [x] Configurable `max_turns_per_state` mapping exists

**Files Impacted:**
- `src/tracker/model.rs` (`RunningEntry`)
- `src/orchestrator/tick.rs` (`run_worker` loop)
- `src/orchestrator/state.rs`
- `src/config/typed.rs`

---

### ISSUE-003: Workflow Terminal State Cleanup on Startup

**Context & Problem:**
On startup, `main.rs` fetches terminal issues and cleans up their workspaces. However, this logic runs **before** the orchestrator starts, and if the tracker fetch fails, the cleanup is silently skipped with a `warn!` log. Over time, this leads to workspace disk leaks.

**Objective:**
Make startup terminal cleanup **mandatory and idempotent**. If it fails, the orchestrator should still start but schedule a background cleanup task.

**Technical Design:**

1. **Idempotent cleanup:**
   - `WorkspaceManager::remove_workspace` already handles missing directories gracefully.
   - Ensure `runner.cleanup_workspace` (Daytona) is also idempotent.

2. **Background cleanup task:**
   - If startup cleanup fails, spawn a `tokio::spawn` task that retries cleanup every 5 minutes for terminal issues.

3. **Workspace directory enumeration:**
   - As a safety net, on startup, enumerate `workspace.root` directories.
   - For each directory matching `SYMPHEO-*`, check if the corresponding issue is still active.
   - If not found in active states, remove the directory.

**Acceptance Criteria:**
- [x] Startup cleanup failures do not block orchestrator startup
- [x] Orphaned workspace directories are detected and removed within 5 minutes
- [x] A metric/log line reports the number of cleaned workspaces

**Files Impacted:**
- `src/main.rs`
- `src/workspace/manager.rs`

---

## Workstream 2: OpenCode Adapter — Resilience, Observability & Robustness

### ISSUE-004: Real-Time Event Streaming into Orchestrator State

**Context & Problem:**
`run_worker` blocks on `runner.run_turn(...).await` until the OpenCode process exits or times out. Only **then** does it drain `event_rx` and update `OrchestratorState`. This means the dashboard shows `turn_count: 0`, `session_id: ""`, and `last_event: null` for the entire duration of a turn (potentially hours). The operator has zero visibility into agent progress.

**Objective:**
Spawn a **concurrent event consumer** that updates `OrchestratorState` in real-time as events arrive.

**Technical Design:**

1. **Concurrent event consumer task:**
   ```rust
   let state_for_events = state.clone();
   let issue_id_for_events = issue.id.clone();
   let event_consumer = tokio::spawn(async move {
       while let Some(event) = event_rx.recv().await {
           update_state_from_event(&state_for_events, &issue_id_for_events, event).await;
       }
   });
   ```

2. **State update logic extraction:**
   - Extract the `match &event { ... }` block from `run_worker` into a standalone async fn:
     ```rust
     async fn apply_agent_event(
         state: &Arc<RwLock<OrchestratorState>>,
         issue_id: &str,
         event: AgentEvent,
     )
     ```
   - This function handles `RateLimit`, `TokenUsage`, `Notification`, `TurnFailed`, and updates `LiveSession` fields (`last_event`, `last_message`, `last_timestamp`, `input_tokens`, `output_tokens`).

3. **Session initialization on `StepStart`:**
   - When the first `StepStart` event arrives, create the `LiveSession` in `RunningEntry` immediately.
   - Set `session_id`, `thread_id`, `turn_id` from the event payload.

4. **Token accounting fix (ISSUE-004b):**
   - On the first `TokenUsage` event for a given turn, compute delta against `0`.
   - Store `last_reported_*` in the `LiveSession` immediately after processing.
   - On subsequent `TokenUsage` events in the **same turn**, compute delta against the stored values.
   - At turn end, reset the per-turn baseline to `0` for the next turn.

**Acceptance Criteria:**
- [x] Dashboard shows `session_id` and `turn_count > 0` within 5 seconds of agent launch
- [x] `last_event` and `last_message` update in real-time as the agent streams output
- [x] `codex_totals` increments in real-time (verified by polling `/api/v1/state`)
- [x] Multiple `TokenUsage` events in a single turn do not double-count
- [x] No regression in `run_worker` control flow (success/failure detection still works)

**Files Impacted:**
- `src/orchestrator/tick.rs` (major refactor of event handling)
- `src/tracker/model.rs` (possible `LiveSession` field additions)

---

### ISSUE-005: Structured Logging with Correlation IDs

**Context & Problem:**
Backend logs (especially `step_finish`) omit the `issue_id`, making it impossible to correlate events when multiple agents run concurrently. The `opencode::stderr` logs also lack context beyond the issue ID injected manually.

**Objective:**
Ensure **every log line** emitted by the adapter carries the `issue_id`, `issue_identifier`, and `session_id` as structured fields.

**Technical Design:**

1. **Span-based tracing:**
   - Wrap `run_turn` and the stdout reader loop in a `tracing::span!`:
     ```rust
     let span = tracing::info_span!(
         "opencode_turn",
         issue_id = %issue.id,
         issue_identifier = %issue.identifier,
         session_id = %session_id.unwrap_or("new"),
     );
     let _enter = span.enter();
     ```
   - All `tracing::info!`, `warn!`, `debug!` inside the span automatically inherit these fields.

2. **Backend log enrichment:**
   - `LocalBackend::run_turn`: add `issue_id` and `issue_identifier` to **every** log line:
     - `launching opencode agent`
     - `step_start` (with session_id, message_id)
     - `step_finish` (with session_id, message_id, reason, success)
     - `text` (truncated, with session_id)
     - stderr warnings (already has issue_id, keep it)

3. **Event serialization in logs:**
   - When an `AgentEvent` is parsed successfully, log it at `DEBUG` level with its full JSON payload (truncated to 1KB).
   - When parsing fails, log the raw line at `WARN` level for debugging.

**Acceptance Criteria:**
- [x] Every log line from `local.rs` contains `issue_id` and `issue_identifier`
- [x] `step_finish` logs include `session_id` and `turn_id`
- [x] `tracing::span` is used so child tasks (stderr reader) inherit context
- [x] A grep for `issue_id=118` returns all log lines related to that issue

**Files Impacted:**
- `src/agent/backend/local.rs`
- `src/agent/backend/daytona.rs` (same treatment)

---

### ISSUE-006: OpenCode Argument Validation & Pre-Flight Check

**Context & Problem:**
SYMPHEO-108 (Doc skill) fails immediately on every retry. The stderr output shows the **complete `opencode --help` text**, suggesting the CLI either did not receive the prompt or rejected the argument vector. The Doc skill is ~110 lines of markdown with backticks, quotes, and bullet points — any of which could interact badly with shell variable expansion or opencode's argument parser.

**Objective:**
Add a **pre-flight check** that validates the `opencode` CLI can accept the constructed command before spawning the real agent. Also improve prompt delivery robustness.

**Technical Design:**

1. **Prompt delivery via file (already done, but enhance):**
   - Keep writing `.sympheo_prompt_{issue_id}.txt`.
   - **Ensure the file is UTF-8 without BOM.**
   - **Strip ANSI escape sequences** if any are present in the rendered prompt.

2. **Pre-flight validation command:**
   - Before the real `run_turn`, execute a lightweight probe:
     ```bash
     opencode run "__sympheo_probe__" --format json --dir "{workspace}" --dangerously-skip-permissions
     ```
   - Expect a `step_finish` or immediate exit with code 0.
   - If the probe returns `--help`, logs an error: *"OpenCode rejected arguments — check prompt length or special characters"*.
   - If probe succeeds, proceed with the real prompt.

3. **Argument length guard:**
   - Measure the prompt byte length.
   - If it exceeds a threshold (e.g., 100KB), log a warning and consider splitting or truncating.
   - (Note: `opencode run` receives the prompt via `$PROMPT` variable, so ARG_MAX is not the issue — the issue is opencode's internal parser.)

4. **Sanitize prompt for opencode:**
   - Remove or escape sequences that look like CLI flags inside the prompt:
     - Replace standalone lines matching `--[a-z-]+` with backtick-wrapped versions.
   - This prevents the agent's own skill text from being misinterpreted as opencode flags.

**Acceptance Criteria:**
- [x] Pre-flight probe runs before every `run_turn`
- [x] If probe shows help text, the turn fails fast with a clear error message
- [x] Prompt length is logged at `DEBUG` level
- [x] SYMPHEO-108 (Doc skill) no longer fails with "turn reported failure" due to argument parsing

**Files Impacted:**
- `src/agent/backend/local.rs`
- `src/agent/backend/daytona.rs` (similar probe if applicable)

---

### ISSUE-007: Graceful Process Termination & Orphan Prevention

**Context & Problem:**
When `sympheo` receives `SIGINT` or is killed, the OpenCode child processes are **not terminated**. They become orphans, consuming CPU/GPU and holding workspace locks. The `kill_process_group` logic only runs when a turn completes or times out inside `run_turn`.

**Objective:**
Ensure **all child processes** (OpenCode, bash, hooks) are reliably terminated when the orchestrator shuts down or when a worker is cancelled.

**Technical Design:**

1. **PID tracking in `RunningEntry`:**
   - Add `agent_pid: Option<u32>` to `LiveSession` (field exists but is unused).
   - Set it immediately after `cmd.spawn()` in `LocalBackend::run_turn`.
   - Send it back to the orchestrator via a new `AgentEvent::ProcessStarted { pid }`.

2. **Global shutdown handler:**
   - In `main.rs`, install a `tokio::signal::ctrl_c()` handler.
   - On shutdown:
     - Set a global `SHUTDOWN` atomic flag.
     - Iterate over `state.running`, read `agent_pid`, and send `SIGTERM` (graceful) then `SIGKILL` (after 5s).
   - Use `libc::killpg` on the process group if available, fallback to `libc::kill(pid, SIGKILL)`.

3. **Cancellation during `run_turn`:**
   - The `cancelled: Arc<AtomicBool>` flag is checked at the top of the worker loop.
   - However, if `run_turn` is blocked waiting for stdout, the worker won't notice cancellation until the process exits.
   - **Fix:** Pass the `cancelled` flag into `LocalBackend::run_turn`. Spawn a watchdog task that polls `cancelled` every second. If `true`, call `kill_process_group` on the child from within the backend.

**Acceptance Criteria:**
- [x] `Ctrl+C` terminates sympheo and all child `opencode` processes within 5 seconds
- [x] `RunningEntry` tracks the agent PID accurately
- [x] Cancellation from `reconcile()` kills the agent process immediately
- [x] No orphan `opencode run` processes remain after sympheo exits

**Files Impacted:**
- `src/main.rs`
- `src/agent/backend/local.rs`
- `src/agent/backend/mod.rs` (`AgentBackend` trait signature may need `cancelled` param)
- `src/agent/runner.rs`
- `src/orchestrator/tick.rs`

---

## Workstream 3: GitHub Tracker — API-Native CRUD & Lifecycle Management

### ISSUE-008: Replace `gh` CLI Dependency with GraphQL Mutations

**Context & Problem:**
The current prompt template instructs the agent to use `gh` CLI to move tickets. This creates a **hidden external dependency** (`gh` must be installed, authenticated, and in PATH). More importantly, `gh` uses the REST API v3 for some operations and GraphQL for others, making behavior inconsistent and hard to debug.

**Objective:**
Implement a **native GitHub GraphQL mutation client** inside `GithubTracker` that supports:
- Moving a project item to a different status (column)
- Adding/updating/removing a comment on an issue
- Creating a pull request linked to an issue
- Updating issue fields (title, body, labels)

**Technical Design:**

1. **GraphQL mutations module:**
   - New file: `src/tracker/github/mutations.rs`
   - Queries:
     ```graphql
     mutation MoveProjectItem($projectId: ID!, $itemId: ID!, $fieldId: ID!, $optionId: String!) {
       updateProjectV2ItemFieldValue(input: {
         projectId: $projectId,
         itemId: $itemId,
         fieldId: $fieldId,
         value: { singleSelectOptionId: $optionId }
       }) {
         projectV2Item { id }
       }
     }
     ```
   - For comments:
     ```graphql
     mutation AddComment($subjectId: ID!, $body: String!) {
       addComment(input: {subjectId: $subjectId, body: $body}) {
         commentEdge { node { id } }
       }
     }
     ```

2. **Field ID caching:**
   - The "Status" field ID and option IDs are stable per project but expensive to query.
   - Cache them in `GithubTracker` in a `HashMap<String, String>` (`field_name -> field_id`, `option_name -> option_id`).
   - Populate cache on first use or during `new()`.

3. **New trait methods:**
   - Extend `IssueTracker` with optional lifecycle methods:
     ```rust
     async fn move_issue_state(&self, issue_id: &str, new_state: &str) -> Result<(), SympheoError>;
     async fn add_comment(&self, issue_id: &str, body: &str) -> Result<(), SympheoError>;
     async fn update_issue_body(&self, issue_id: &str, body: &str) -> Result<(), SympheoError>;
     ```
   - Provide default no-op implementations for backward compatibility with other trackers.

4. **Remove `gh` from prompt template:**
   - Update the base prompt to instruct the agent to use the **orchestrator API** (or a wrapper script) instead of `gh`.
   - Alternatively, provide a thin wrapper script `sympheo-gh-bridge` that calls the orchestrator's internal APIs.

**Acceptance Criteria:**
- [ ] `GithubTracker` can move a project item to any active or terminal state via GraphQL
- [ ] `GithubTracker` can post comments on issues via GraphQL
- [ ] Field IDs are cached and not re-fetched on every mutation
- [ ] The prompt template no longer references `gh` CLI
- [ ] All mutations handle rate limits (`X-RateLimit-Remaining`) with exponential backoff

**Files Impacted:**
- `src/tracker/mod.rs` (trait extension)
- `src/tracker/github.rs` (major refactor)
- `src/tracker/github/mutations.rs` (new)
- `WORKFLOW.md` (prompt template)

---

### ISSUE-009: Implement `blocked_by` Extraction from GitHub

**Context & Problem:**
`normalize_item()` hardcodes `blocked_by: vec![]`. The `is_blocked()` logic exists but is never exercised with real GitHub data. This makes the "blocked todo" feature purely theoretical.

**Objective:**
Fetch linked items (blocking relationships) from the GitHub Project and populate `blocked_by`.

**Technical Design:**

1. **GraphQL query extension:**
   - Add to the existing `fetch_project_items` query:
     ```graphql
     linkedItems(first: 20) {
       nodes {
         ... on Issue {
           id
           number
           state
         }
       }
     }
     ```
   - Note: GitHub Projects v2 uses `linkedItems` (sub-issues / tracked-by relationships) or `trackedIssues`.

2. **Normalization:**
   - For each linked item that is an `Issue`, extract `id` (or `number` as string) and push to `blocked_by`.
   - The `is_blocked()` check compares blocker states against `terminal_states`.

3. **Performance consideration:**
   - Linked items add GraphQL complexity. If a project has many items with many links, this could hit rate limits.
   - Make linked item fetching **opt-in** via config:
     ```yaml
     tracker:
       fetch_blocked_by: true  # default: false
     ```

**Acceptance Criteria:**
- [x] When `fetch_blocked_by: true`, `Issue::blocked_by` is populated from GitHub
- [x] A ticket in `todo` with an active blocker is skipped by the orchestrator
- [x] Once the blocker reaches a terminal state, the ticket becomes eligible on the next tick

**Files Impacted:**
- `src/tracker/github.rs`
- `src/tracker/model.rs`
- `src/config/typed.rs`

---

### ISSUE-010: Robust Status Extraction (Case-Insensitive Field Matching)

**Context & Problem:**
`extract_status()` looks for a field **literally** named `"Status"` (case-sensitive). If a user renames the field to "State", "Etat", or uses a non-English GitHub interface, status extraction falls back to the raw issue `OPEN`/`CLOSED` state, which breaks the workflow mapping.

**Objective:**
Support case-insensitive and configurable status field names.

**Technical Design:**

1. **Config-driven field name:**
   ```yaml
   tracker:
     status_field_name: "Status"  # default, but overridable
   ```

2. **Case-insensitive matching:**
   - In `extract_status`, compare `field.name.to_lowercase()` against `config.status_field_name().to_lowercase()`.

3. **Fallback strategy:**
   - If no field matches, try heuristics: look for any single-select field whose options match the configured `active_states` / `terminal_states`.
   - Log a warning with the available field names to help debugging.

**Acceptance Criteria:**
- [ ] Status extraction works when the field is named "state", "STATUS", etc.
- [ ] Configurable `status_field_name` is documented
- [ ] A clear warning is logged when the status field cannot be found

**Files Impacted:**
- `src/tracker/github.rs`
- `src/config/typed.rs`
- `docs/04-configuration.md`

---

### ISSUE-011: PR & Branch Lifecycle Integration

**Context & Problem:**
The current workflow mentions *"open a PR and move to Done"* in the Doc stage, but there is **no technical integration** with pull requests. The agent is expected to manually open a PR via `gh` or the web UI, with no tracking or validation from sympheo.

**Objective:**
Integrate PR lifecycle into the orchestrator so that:
- The agent can request a PR creation via a structured event
- The orchestrator validates the PR exists before allowing the ticket to move to `Done`
- Branches are tracked per issue

**Technical Design:**

1. **New tracker methods:**
   ```rust
   async fn create_pull_request(
       &self,
       issue_id: &str,
       title: &str,
       body: &str,
       head_branch: &str,
       base_branch: &str,
   ) -> Result<PullRequest, SympheoError>;

   async fn get_linked_prs(&self, issue_id: &str) -> Result<Vec<PullRequest>, SympheoError>;
   ```

2. **New `AgentEvent` variants:**
   - `AgentEvent::CreatePullRequest { title, head, base }`
   - The orchestrator intercepts this event, calls the tracker, and injects the PR URL back into the next turn's prompt.

3. **Branch name convention:**
   - Auto-generate branch names: `sympheo/{issue_number}-{slug}`
   - Store `branch_name` in `Issue` and persist it across turns.

4. **Validation gate for `Done`:**
   - Before allowing a ticket to reach `Done`, verify:
     - A linked PR exists (for `in progress` → `review` flow)
     - OR the `doc` skill explicitly waives the PR requirement (for pure doc changes).

**Acceptance Criteria:**
- [ ] `AgentEvent::CreatePullRequest` is defined and parseable
- [ ] Orchestrator can create a PR via GitHub GraphQL/REST
- [ ] PR URL is injected into the next prompt turn
- [ ] Ticket cannot reach `Done` without a linked PR (unless waived)

**Files Impacted:**
- `src/agent/parser.rs`
- `src/tracker/mod.rs`
- `src/tracker/github.rs`
- `src/tracker/model.rs`

---

## Workstream 4: Git Adapter — Decoupled Local SCM Operations

### ISSUE-012: Extract Git Operations into Dedicated `GitAdapter`

**Context & Problem:**
Git operations are currently scattered across:
- `hooks.after_create` (bash `git clone`)
- The agent itself (running `git checkout -b`, `git commit`, `git push` via shell)
- No abstraction means no validation, no retry logic, and no testability.

**Objective:**
Create a **first-class `GitAdapter`** trait and a `LocalGitAdapter` implementation that encapsulates all SCM operations needed by the workflow.

**Technical Design:**

1. **Trait definition:**
   ```rust
   #[async_trait]
   pub trait GitAdapter: Send + Sync {
       async fn clone(&self, url: &str, path: &Path) -> Result<(), SympheoError>;
       async fn checkout_branch(&self, path: &Path, branch: &str, create: bool) -> Result<(), SympheoError>;
       async fn commit(&self, path: &Path, message: &str, files: &[&str]) -> Result<String, SympheoError>; // returns commit hash
       async fn push(&self, path: &Path, remote: &str, branch: &str) -> Result<(), SympheoError>;
       async fn fetch(&self, path: &Path, remote: &str) -> Result<(), SympheoError>;
       async fn merge(&self, path: &Path, branch: &str, strategy: MergeStrategy) -> Result<(), SympheoError>;
       async fn status(&self, path: &Path) -> Result<GitStatus, SympheoError>;
       async fn log(&self, path: &Path, n: usize) -> Result<Vec<CommitInfo>, SympheoError>;
   }
   ```

2. **Implementation:**
   - `LocalGitAdapter` spawns `git` subprocesses (like the current local backend) but with structured output parsing.
   - Use `tokio::process::Command` with `--porcelain` flags for machine-readable output.
   - Capture stderr and map git errors to `SympheoError::GitError`.

3. **Integration with WorkspaceManager:**
   - `WorkspaceManager` holds an `Arc<dyn GitAdapter>`.
   - The `after_create` hook is replaced by a first-class call: `git_adapter.clone(repo_url, &workspace_path).await`.

4. **Testability:**
   - Provide a `MockGitAdapter` for unit tests that simulates success/failure scenarios.

**Acceptance Criteria:**
- [x] `GitAdapter` trait exists in `src/git/adapter.rs`
- [ ] `LocalGitAdapter` exists in `src/git/local.rs`
- [ ] `WorkspaceManager` uses `GitAdapter` instead of bash hooks for clone
- [ ] All git operations return structured errors (not just "exit code 1")
- [ ] `MockGitAdapter` is available for tests

**Files Impacted:**
- `src/git/mod.rs` (new module)
- `src/git/adapter.rs` (new)
- `src/git/local.rs` (new)
- `src/workspace/manager.rs`
- `src/main.rs`

---

### ISSUE-013: Branch Lifecycle Management per Issue

**Context & Problem:**
The prompt tells the agent to *"Work in a dedicated branch from now on"* (In Progress stage), but there is no mechanism to:
- Auto-create the branch
- Track which branch belongs to which issue
- Ensure the branch is pushed before PR creation

**Objective:**
Automate branch creation and tracking as part of the orchestrator lifecycle.

**Technical Design:**

1. **Branch naming convention:**
   - Template: `sympheo/{issue_number}-{sanitized_title}`
   - Example: `sympheo/118-add-ticket-summary-dashboard`
   - Max length: 50 chars to avoid git warnings.

2. **Lifecycle hooks (orchestrator-driven, not bash):**
   - When an issue moves from `spec` → `in progress`:
     - `git_adapter.checkout_branch(&workspace, &branch_name, true).await`
     - `git_adapter.push(&workspace, "origin", &branch_name).await` (set upstream)
   - Store `branch_name` in `Issue` and sync it to the tracker if supported.

3. **Branch protection awareness:**
   - If `push` fails due to branch protection rules, log a warning and instruct the agent to open a PR from the local branch without pushing.

4. **Cleanup:**
   - On terminal state, optionally delete the remote branch (configurable).

**Acceptance Criteria:**
- [x] Branch is auto-created when entering `in progress`
- [x] Branch name is deterministic and stored in `Issue::branch_name`
- [x] Branch is pushed to origin with upstream tracking
- [x] Agent prompt includes the branch name for reference
- [x] Remote branch deletion on terminal state is configurable

**Files Impacted:**
- `src/git/adapter.rs`
- `src/git/local.rs`
- `src/orchestrator/tick.rs` (transition detection)
- `src/tracker/model.rs`

---

### ISSUE-014: Workspace Isolation & Git State Verification

**Context & Problem:**
`WorkspaceManager::create_or_reuse` reuses existing directories. If a previous agent run left the git working tree in a dirty state (uncommitted changes, detached HEAD), the next agent inherits a broken state.

**Objective:**
Ensure every new turn starts from a **known clean git state**.

**Technical Design:**

1. **Pre-run git verification:**
   - Before `before_run` hook, call `git_adapter.status(&workspace)`.
   - If dirty:
     - Option A: `git stash push -m "sympheo-auto-stash-{issue_id}-{timestamp}"`
     - Option B: Hard reset to `origin/main` (destructive, but deterministic).
   - Configurable via:
     ```yaml
     workspace:
       git_reset_strategy: "stash"  # or "hard_reset"
     ```

2. **Detached HEAD detection:**
   - If the workspace is in detached HEAD, checkout the issue's tracked branch or `main`.

3. **Remote sync:**
   - `git fetch origin` before each turn to ensure the agent works on the latest base.

**Acceptance Criteria:**
- [x] Dirty workspaces are detected before agent dispatch
- [x] Stash or reset is applied based on config
- [x] Detached HEAD is automatically resolved
- [x] `git fetch origin` runs before each turn

**Files Impacted:**
- `src/git/adapter.rs`
- `src/git/local.rs`
- `src/workspace/manager.rs`
- `src/config/typed.rs`

---

## Cross-Cutting Concerns

### ISSUE-015: Configuration Hot-Reload Validation

**Context & Problem:**
When `WORKFLOW.md` is modified, the file watcher reloads config and skills **without** calling `validate_for_dispatch()`. An invalid config (e.g., missing `api_key`, malformed skill paths) can be hot-loaded and will only fail on the next tick.

**Objective:**
Validate reloaded config before applying it. If invalid, keep the previous config and log an error.

**Acceptance Criteria:**
- [ ] `validate_for_dispatch()` is called on reloaded config
- [ ] Invalid reloads are rejected with a clear error log
- [ ] Previous config remains active

**Files Impacted:**
- `src/main.rs` (watcher callback)

---

### ISSUE-016: Metrics & Health Endpoint

**Context & Problem:**
The only observability is the HTML dashboard and `/api/v1/state`. There is no Prometheus-compatible metrics endpoint, no health check, and no structured event log for external SIEM integration.

**Objective:**
Add a `/health` endpoint and expose key counters as metrics.

**Technical Design:**
- `GET /health` → `200 OK` with JSON: `{ "status": "ok", "last_tick_at": "...", "running": N, "retrying": M }`
- `GET /metrics` → Prometheus text format:
  ```
  sympheo_running_agents 2
  sympheo_retry_queue_size 1
  sympheo_total_tokens{direction="input"} 15000
  sympheo_total_tokens{direction="output"} 4200
  sympheo_tick_duration_ms 450
  ```

**Acceptance Criteria:**
- [ ] `/health` returns 200 when the tick loop is active
- [ ] `/metrics` returns Prometheus-compatible counters
- [ ] Metrics survive config reloads

**Files Impacted:**
- `src/server/mod.rs`

---

## Appendix A: Event Schema (OpenCode)

The OpenCode CLI emits JSON lines on stdout when `--format json` is used. Sympheo currently parses a subset. The full schema that SHOULD be supported:

```json
{"type":"step_start","timestamp":1715175600,"sessionID":"sess-abc","part":{"id":"p1","messageID":"msg-1","sessionID":"sess-abc","type":"step"}}
{"type":"text","timestamp":1715175601,"sessionID":"sess-abc","part":{"id":"p2","messageID":"msg-2","sessionID":"sess-abc","type":"text","text":"Analyzing..."}}
{"type":"step_finish","timestamp":1715175700,"sessionID":"sess-abc","part":{"id":"p3","reason":"stop","messageID":"msg-3","sessionID":"sess-abc","type":"finish","tokens":{"total":100,"input":50,"output":40,"reasoning":10,"cache":{"write":5,"read":3}}}}
```

**Future events to support:**
- `type: "tool_call"` — agent invokes a tool (file read, write, shell)
- `type: "tool_result"` — tool response
- `type: "approval_request"` — agent requests user confirmation (should trigger auto-approval or pause)

---

## Appendix B: GitHub GraphQL Mutations Reference

### Move Project Item
```graphql
mutation($projectId: ID!, $itemId: ID!, $fieldId: ID!, $optionId: String!) {
  updateProjectV2ItemFieldValue(input: {
    projectId: $projectId,
    itemId: $itemId,
    fieldId: $fieldId,
    value: { singleSelectOptionId: $optionId }
  }) {
    projectV2Item { id }
  }
}
```

### Add Comment
```graphql
mutation($subjectId: ID!, $body: String!) {
  addComment(input: {subjectId: $subjectId, body: $body}) {
    commentEdge { node { id } }
  }
}
```

### Create Pull Request
```graphql
mutation($repositoryId: ID!, $baseRefName: String!, $headRefName: String!, $title: String!, $body: String) {
  createPullRequest(input: {
    repositoryId: $repositoryId,
    baseRefName: $baseRefName,
    headRefName: $headRefName,
    title: $title,
    body: $body
  }) {
    pullRequest { id url }
  }
}
```

---

## Priority Matrix

| Issue | Workstream | Priority | Effort | Impact |
|-------|-----------|----------|--------|--------|
| ISSUE-001 | Workflow | P0 | Low | 🔥 Unblocks entire lifecycle |
| ISSUE-004 | OpenCode | P0 | High | 🔥 Makes system observable |
| ISSUE-008 | GitHub | P0 | High | 🔥 Removes hidden `gh` dependency |
| ISSUE-012 | Git | P0 | Medium | 🔥 Enables testable SCM |
| ISSUE-007 | OpenCode | P1 | Medium | 🛡️ Prevents resource leaks |
| ISSUE-002 | Workflow | P1 | Medium | 🛡️ Prevents infinite loops |
| ISSUE-005 | OpenCode | P1 | Low | 🛡️ Debuggability |
| ISSUE-006 | OpenCode | P1 | Medium | 🛡️ Robustness |
| ISSUE-009 | GitHub | P2 | Low | 🚀 Feature parity |
| ISSUE-010 | GitHub | P2 | Low | 🚀 I18n robustness |
| ISSUE-011 | GitHub | P2 | High | 🚀 Full PR lifecycle |
| ISSUE-013 | Git | P2 | Medium | 🚀 Branch automation |
| ISSUE-014 | Git | P2 | Medium | 🛡️ State hygiene |
| ISSUE-003 | Workflow | P3 | Low | 🛡️ Cleanup |
| ISSUE-015 | Config | P3 | Low | 🛡️ Safety |
| ISSUE-016 | Observability | P3 | Medium | 🚀 Production readiness |

---

*End of Document*
