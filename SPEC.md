# Sympheo Service Specification

Status: Draft v1 (language-agnostic)

Purpose: Define a service that orchestrates coding-agent CLIs to get project work done.

## Normative Language

The key words `MUST`, `MUST NOT`, `REQUIRED`, `SHOULD`, `SHOULD NOT`, `RECOMMENDED`, `MAY`, and
`OPTIONAL` in this document are to be interpreted as described in RFC 2119.

`Implementation-defined` means the behavior is part of the implementation contract, but this
specification does not prescribe one universal policy. Implementations MUST document the selected
behavior.

## 1. Problem Statement

Sympheo is a long-running automation service that continuously reads work from an issue tracker,
creates an isolated workspace for each issue, and runs a coding-agent CLI session for that issue
inside the workspace.

The service solves four operational problems:

- It turns issue execution into a repeatable daemon workflow instead of manual scripts.
- It isolates agent execution in per-issue workspaces so agent commands run only inside per-issue
  workspace directories.
- It keeps the workflow policy in-repo (`WORKFLOW.md`) so teams version the agent prompt and runtime
  settings with their code.
- It provides enough observability to operate and debug multiple concurrent agent runs.

Sympheo is designed to be **tracker-agnostic** and **CLI-agnostic**. Both the issue tracker
integration and the coding-agent CLI integration are abstracted behind adapter contracts. This
specification defines:

- A **GitHub Projects (v2)** tracker adapter as the first-class reference adapter.
- A **Linear** tracker adapter as an extension (Appendix B).
- An **OpenCode** CLI adapter as the first-class reference adapter.
- A generic CLI adapter contract that other coding-agent CLIs (for example `pi.dev`, future tools)
  MAY implement.

Implementations are expected to document their trust and safety posture explicitly. This
specification does not require a single approval, sandbox, or operator-confirmation policy; some
implementations target trusted environments with a high-trust configuration, while others require
stricter approvals or sandboxing.

Important boundary:

- Sympheo is a scheduler/runner and tracker reader.
- Ticket writes (state transitions, comments, PR links) are typically performed by the coding agent
  using tools available in the workflow/runtime environment.
- A successful run can end at a workflow-defined handoff state (for example `Human Review`), not
  necessarily a tracker terminal state.

## 2. Goals and Non-Goals

### 2.1 Goals

- Poll the issue tracker on a fixed cadence and dispatch work with bounded concurrency.
- Maintain a single authoritative orchestrator state for dispatch, retries, and reconciliation.
- Create deterministic per-issue workspaces and preserve them across runs.
- Stop active runs when issue state changes make them ineligible.
- Recover from transient failures with exponential backoff.
- Load runtime behavior from a repository-owned `WORKFLOW.md` contract.
- Expose operator-visible observability (at minimum structured logs).
- Support tracker/filesystem-driven restart recovery without requiring a persistent database; exact
  in-memory scheduler state is not restored.
- Decouple tracker integration and coding-agent CLI integration behind adapter contracts so
  contributors can add new trackers and CLIs without modifying the orchestrator core.

### 2.2 Non-Goals

- Rich web UI or multi-tenant control plane.
- Prescribing a specific dashboard or terminal UI implementation.
- General-purpose workflow engine or distributed job scheduler.
- Built-in business logic for how to edit tickets, PRs, or comments. (That logic lives in the
  workflow prompt and agent tooling.)
- Mandating strong sandbox controls beyond what the coding-agent CLI and host OS provide.
- Mandating a single default approval, sandbox, or operator-confirmation posture for all
  implementations.
- Validating agent-claimed outcomes (Sympheo does not verify that artifacts exist, tests pass, or
  PRs are linked before the agent transitions a ticket).

## 3. System Overview

### 3.1 Main Components

1. `Workflow Loader`
   - Reads `WORKFLOW.md`.
   - Parses YAML front matter and prompt body.
   - Returns `{config, prompt_template}`.

2. `Config Layer`
   - Exposes typed getters for workflow config values.
   - Applies defaults and environment variable indirection.
   - Performs validation used by the orchestrator before dispatch.

3. `Tracker Adapter` (pluggable)
   - Implements the Tracker Adapter Contract (Section 11).
   - Fetches candidate issues in active states.
   - Fetches current states for specific issue IDs (reconciliation).
   - Fetches terminal-state issues during startup cleanup.
   - Normalizes tracker payloads into a stable issue model.
   - Reference adapter: GitHub Projects (v2). Extension adapter: Linear.

4. `Orchestrator`
   - Owns the poll tick.
   - Owns the in-memory runtime state.
   - Decides which issues to dispatch, retry, stop, or release.
   - Tracks session metrics and retry queue state.

5. `Workspace Manager`
   - Maps issue identifiers to workspace paths.
   - Ensures per-issue workspace directories exist.
   - Runs workspace lifecycle hooks.
   - Cleans workspaces for terminal issues.

6. `CLI Adapter` (pluggable)
   - Implements the CLI Adapter Contract (Section 10).
   - Creates workspace context.
   - Builds prompt from issue + workflow template.
   - Launches the coding-agent CLI subprocess (one invocation per turn).
   - Parses CLI stdout/logs to extract message, status, and token usage.
   - Streams normalized agent updates back to the orchestrator.
   - Reference adapter: OpenCode (`opencode run`).

7. `Status Surface` (OPTIONAL)
   - Presents human-readable runtime status (for example terminal output, dashboard, or other
     operator-facing view).

8. `Logging`
   - Emits structured runtime logs to one or more configured sinks.

### 3.2 Abstraction Levels

Sympheo is easiest to port when kept in these layers:

1. `Policy Layer` (repo-defined)
   - `WORKFLOW.md` prompt body.
   - Team-specific rules for ticket handling, validation, and handoff.

2. `Configuration Layer` (typed getters)
   - Parses front matter into typed runtime settings.
   - Handles defaults, environment tokens, and path normalization.

3. `Coordination Layer` (orchestrator)
   - Polling loop, issue eligibility, concurrency, retries, reconciliation.

4. `Execution Layer` (workspace + agent CLI subprocess)
   - Filesystem lifecycle, workspace preparation, CLI adapter invocation.

5. `Integration Layer` (tracker adapter + CLI adapter)
   - Tracker API calls and normalization.
   - CLI subprocess management and output parsing.

6. `Observability Layer` (logs + OPTIONAL status surface)
   - Operator visibility into orchestrator and agent behavior.

### 3.3 External Dependencies

- Issue tracker API (GitHub GraphQL API for `tracker.kind: github` in this specification version;
  Linear GraphQL API for the Linear extension).
- Local filesystem for workspaces and logs.
- OPTIONAL workspace population tooling (for example Git CLI, if used).
- Coding-agent CLI executable (for example `opencode`) that satisfies the CLI Adapter Contract.
- Host environment authentication for the issue tracker and coding-agent CLI.

## 4. Core Domain Model

### 4.1 Entities

#### 4.1.1 Issue

Normalized issue record used by orchestration, prompt rendering, and observability output.

Fields:

- `id` (string)
  - Stable tracker-internal ID (for GitHub: the GraphQL node ID of the issue).
- `identifier` (string)
  - Human-readable ticket key.
  - For GitHub: `<repo>#<number>` (example: `sympheo#42`).
  - For Linear: `<TEAM>-<NUMBER>` (example: `ABC-123`).
- `title` (string)
- `description` (string or null)
- `priority` (integer or null)
  - Lower numbers are higher priority in dispatch sorting.
  - For GitHub: derived from a configured ProjectV2 custom field (see Section 11.4).
  - For Linear: native priority value.
- `state` (string)
  - Current tracker state name.
  - For GitHub: value of the configured ProjectV2 status field.
  - For Linear: workflow state name.
- `branch_name` (string or null)
  - For GitHub: generated from `identifier` and `title` (see Section 4.2).
  - For Linear: tracker-provided branch metadata if available.
- `url` (string or null)
- `labels` (list of strings)
  - Normalized to lowercase.
- `blocked_by` (list of blocker refs)
  - Each blocker ref contains:
    - `id` (string or null)
    - `identifier` (string or null)
    - `state` (string or null)
- `created_at` (timestamp or null)
- `updated_at` (timestamp or null)

#### 4.1.2 Workflow Definition

Parsed `WORKFLOW.md` payload:

- `config` (map)
  - YAML front matter root object.
- `prompt_template` (string)
  - Markdown body after front matter, trimmed.

#### 4.1.3 Service Config (Typed View)

Typed runtime values derived from `WorkflowDefinition.config` plus environment resolution.

Examples:

- poll interval
- workspace root
- active and terminal issue states
- concurrency limits
- coding-agent CLI executable/args/timeouts
- workspace hooks

#### 4.1.4 Workspace

Filesystem workspace assigned to one issue identifier.

Fields (logical):

- `path` (absolute workspace path)
- `workspace_key` (sanitized issue identifier)
- `created_now` (boolean, used to gate `after_create` hook)

#### 4.1.5 Run Attempt

One execution attempt for one issue.

Fields (logical):

- `issue_id`
- `issue_identifier`
- `attempt` (integer or null, `null` for first run, `>=1` for retries/continuation)
- `workspace_path`
- `started_at`
- `status`
- `error` (OPTIONAL)

#### 4.1.6 Live Session (Agent Session Metadata)

State tracked while a coding-agent CLI worker is running.

Fields:

- `session_id` (string, opaque identifier produced by the CLI adapter; see Section 10.2)
- `agent_session_handle` (string or null)
  - CLI-adapter-provided handle used to resume the same session across turns (for OpenCode: the
    `--session` value).
- `agent_pid` (integer or null)
  - PID of the currently running CLI subprocess turn. May be `null` between turns.
- `last_agent_event` (string/enum or null)
- `last_agent_timestamp` (timestamp or null)
- `last_agent_message` (summarized payload)
- `agent_input_tokens` (integer)
- `agent_output_tokens` (integer)
- `agent_total_tokens` (integer)
- `last_reported_input_tokens` (integer)
- `last_reported_output_tokens` (integer)
- `last_reported_total_tokens` (integer)
- `turn_count` (integer)
  - Number of coding-agent turns started within the current worker lifetime.

#### 4.1.7 Retry Entry

Scheduled retry state for an issue.

Fields:

- `issue_id`
- `identifier` (best-effort human ID for status surfaces/logs)
- `attempt` (integer, 1-based for retry queue)
- `due_at_ms` (monotonic clock timestamp)
- `timer_handle` (runtime-specific timer reference)
- `error` (string or null)

#### 4.1.8 Orchestrator Runtime State

Single authoritative in-memory state owned by the orchestrator.

Fields:

- `poll_interval_ms` (current effective poll interval)
- `max_concurrent_agents` (current effective global concurrency limit)
- `running` (map `issue_id -> running entry`)
- `claimed` (set of issue IDs reserved/running/retrying)
- `retry_attempts` (map `issue_id -> RetryEntry`)
- `completed` (set of issue IDs; bookkeeping only, not dispatch gating)
- `agent_totals` (aggregate tokens + runtime seconds)
- `agent_rate_limits` (latest rate-limit snapshot from agent events, if exposed by the CLI adapter)

### 4.2 Stable Identifiers and Normalization Rules

- `Issue ID`
  - Use for tracker lookups and internal map keys.
- `Issue Identifier`
  - Use for human-readable logs and workspace naming.
  - For GitHub: `<repo>#<number>` (example: `sympheo#42`).
  - For Linear: `<TEAM>-<NUMBER>` (example: `ABC-123`).
- `Workspace Key`
  - Derive from `issue.identifier` by replacing any character not in `[A-Za-z0-9._]` with `-`.
  - For GitHub identifiers, this turns `sympheo#42` into `sympheo-42`.
  - For Linear identifiers, this turns `ABC-123` into `ABC-123` (already valid).
  - Use the sanitized value for the workspace directory name.
- `Normalized Issue State`
  - Compare states after `lowercase`.
- `Branch Name (GitHub)`
  - Generated from `<number>-<slugified-title>` where `slugified-title` is `lowercase`, replaces
    any non-`[a-z0-9]` run with `-`, trims leading/trailing `-`, and truncates to a reasonable
    length (RECOMMENDED: 60 characters).
- `Session ID`
  - Opaque string produced by the CLI adapter. The adapter MUST guarantee uniqueness across
    concurrent runs within one Sympheo process. RECOMMENDED format:
    `<adapter_kind>-<adapter_session_handle>-<turn_number>`.

## 5. Workflow Specification (Repository Contract)

### 5.1 File Discovery and Path Resolution

Workflow file path precedence:

1. Explicit application/runtime setting (set by CLI startup path).
2. Default: `WORKFLOW.md` in the current process working directory.

Loader behavior:

- If the file cannot be read, return `missing_workflow_file` error.
- The workflow file is expected to be repository-owned and version-controlled.

### 5.2 File Format

`WORKFLOW.md` is a Markdown file with OPTIONAL YAML front matter.

Design note:

- `WORKFLOW.md` SHOULD be self-contained enough to describe and run different workflows (prompt,
  runtime settings, hooks, tracker selection/config, and CLI adapter selection/config) without
  requiring out-of-band service-specific configuration.

Parsing rules:

- If file starts with `---`, parse lines until the next `---` as YAML front matter.
- Remaining lines become the prompt body.
- If front matter is absent, treat the entire file as prompt body and use an empty config map.
- YAML front matter MUST decode to a map/object; non-map YAML is an error.
- Prompt body is trimmed before use.

Returned workflow object:

- `config`: front matter root object (not nested under a `config` key).
- `prompt_template`: trimmed Markdown body.

### 5.3 Front Matter Schema

Top-level keys:

- `tracker`
- `polling`
- `workspace`
- `hooks`
- `agent`
- `cli`

Unknown keys SHOULD be ignored for forward compatibility.

Note:

- The workflow front matter is extensible. Extensions MAY define additional top-level keys without
  changing the core schema above.
- Extensions SHOULD document their field schema, defaults, validation rules, and whether changes
  apply dynamically or require restart.

#### 5.3.1 `tracker` (object)

Fields common to all tracker kinds:

- `kind` (string)
  - REQUIRED for dispatch.
  - Reference value: `github`
  - Extension value: `linear` (see Appendix B)
- `active_states` (list of strings)
  - Default: implementation-defined; RECOMMENDED `["Todo", "In Progress"]`.
- `terminal_states` (list of strings)
  - Default: implementation-defined; RECOMMENDED `["Done", "Cancelled"]`.

Tracker-kind-specific fields are defined by the corresponding adapter section. See Section 11.4
for GitHub and Appendix B for Linear.

#### 5.3.2 `polling` (object)

Fields:

- `interval_ms` (integer)
  - Default: `30000`
  - Changes SHOULD be re-applied at runtime and affect future tick scheduling without restart.

#### 5.3.3 `workspace` (object)

Fields:

- `root` (path string or `$VAR`)
  - Default: `<system-temp>/sympheo_workspaces`
  - `~` is expanded.
  - Relative paths are resolved relative to the directory containing `WORKFLOW.md`.
  - The effective workspace root is normalized to an absolute path before use.

#### 5.3.4 `hooks` (object)

Fields:

- `after_create` (multiline shell script string, OPTIONAL)
  - Runs only when a workspace directory is newly created.
  - Failure aborts workspace creation.
- `before_run` (multiline shell script string, OPTIONAL)
  - Runs before each agent attempt after workspace preparation and before launching the coding
    agent.
  - Failure aborts the current attempt.
- `after_run` (multiline shell script string, OPTIONAL)
  - Runs after each agent attempt (success, failure, timeout, or cancellation) once the workspace
    exists.
  - Failure is logged but ignored.
- `before_remove` (multiline shell script string, OPTIONAL)
  - Runs before workspace deletion if the directory exists.
  - Failure is logged but ignored; cleanup still proceeds.
- `timeout_ms` (integer, OPTIONAL)
  - Default: `60000`
  - Applies to all workspace hooks.
  - Invalid values fail configuration validation.
  - Changes SHOULD be re-applied at runtime for future hook executions.

#### 5.3.5 `agent` (object)

Fields:

- `max_concurrent_agents` (integer)
  - Default: `10`
  - Changes SHOULD be re-applied at runtime and affect subsequent dispatch decisions.
- `max_turns` (positive integer)
  - Default: `20`
  - Limits the number of coding-agent turns within one worker session.
  - Invalid values fail configuration validation.
- `max_retry_backoff_ms` (integer)
  - Default: `300000` (5 minutes)
  - Changes SHOULD be re-applied at runtime and affect future retry scheduling.
- `max_concurrent_agents_by_state` (map `state_name -> positive integer`)
  - Default: empty map.
  - State keys are normalized (`lowercase`) for lookup.
  - Invalid entries (non-positive or non-numeric) are ignored.

#### 5.3.6 `cli` (object)

The `cli` object configures the coding-agent CLI adapter. Sympheo selects the CLI adapter based on
the `cli.command` value (see Section 10.1). Fields are intentionally a flat generic schema; each
CLI adapter reads what it needs and ignores the rest.

Common fields:

- `command` (shell command string)
  - REQUIRED.
  - The runtime launches this command via `bash -lc` in the workspace directory.
  - Reference adapter (OpenCode) default: `opencode run`.
- `args` (list of strings, OPTIONAL)
  - Additional arguments appended to `command` for each turn invocation.
  - Adapters MAY add their own arguments in front of these (for example session resumption flags).
- `env` (map `string -> string`, OPTIONAL)
  - Environment variables added to the subprocess environment for each turn invocation.
  - Values support `$VAR_NAME` indirection.
- `turn_timeout_ms` (integer)
  - Default: `3600000` (1 hour)
  - Total wall-clock timeout for a single turn invocation.
- `read_timeout_ms` (integer)
  - Default: `5000`
  - Timeout for individual read operations on the CLI subprocess output, when the adapter parses
    streaming output.
- `stall_timeout_ms` (integer)
  - Default: `300000` (5 minutes)
  - Maximum allowed silence between two parsed CLI events before the orchestrator kills the run.
  - If `<= 0`, stall detection is disabled.
- `options` (map, OPTIONAL)
  - Adapter-specific opaque options. Adapters MAY define their own keys (for example
    `options.model`, `options.permissions`, `options.approval_policy`). Sympheo does not
    interpret this map; it is forwarded verbatim to the adapter.

Adapter selection rule:

- If a workflow specifies `cli.command`, Sympheo inspects the command's leading binary token to
  select an adapter (for example `opencode` -> OpenCode adapter, `pi` -> pi.dev adapter).
- If no adapter matches, Sympheo MUST fail dispatch validation with a typed error
  (`cli_adapter_not_found`).
- Implementations MAY allow operators to register custom adapters at startup.

### 5.4 Prompt Template Contract

The Markdown body of `WORKFLOW.md` is the per-issue prompt template.

Rendering requirements:

- Use a strict template engine (Liquid-compatible semantics are sufficient).
- Unknown variables MUST fail rendering.
- Unknown filters MUST fail rendering.

Template input variables:

- `issue` (object)
  - Includes all normalized issue fields, including labels and blockers.
- `attempt` (integer or null)
  - `null`/absent on first attempt.
  - Integer on retry or continuation run.

Fallback prompt behavior:

- If the workflow prompt body is empty, the runtime MAY use a minimal default prompt
  (`You are working on an issue from the configured tracker.`).
- Workflow file read/parse failures are configuration/validation errors and SHOULD NOT silently
  fall back to a prompt.

### 5.5 Workflow Validation and Error Surface

Error classes:

- `missing_workflow_file`
- `workflow_parse_error`
- `workflow_front_matter_not_a_map`
- `template_parse_error` (during prompt rendering)
- `template_render_error` (unknown variable/filter, invalid interpolation)
- `cli_adapter_not_found`

Dispatch gating behavior:

- Workflow file read/YAML errors block new dispatches until fixed.
- Template errors fail only the affected run attempt.
- Missing CLI adapter blocks new dispatches until fixed.

## 6. Configuration Specification

### 6.1 Configuration Resolution Pipeline

Configuration is resolved in this order:

1. Select the workflow file path (explicit runtime setting, otherwise cwd default).
2. Parse YAML front matter into a raw config map.
3. Apply built-in defaults for missing OPTIONAL fields.
4. Resolve `$VAR_NAME` indirection only for config values that explicitly contain `$VAR_NAME`.
5. Coerce and validate typed values.
6. Resolve the tracker adapter from `tracker.kind`.
7. Resolve the CLI adapter from `cli.command`.

Environment variables do not globally override YAML values. They are used only when a config value
explicitly references them.

Value coercion semantics:

- Path/command fields support:
  - `~` home expansion
  - `$VAR` expansion for env-backed path values
  - Apply expansion only to values intended to be local filesystem paths; do not rewrite URIs or
    arbitrary shell command strings.
- Relative `workspace.root` values resolve relative to the directory containing the selected
  `WORKFLOW.md`.

### 6.2 Dynamic Reload Semantics

Dynamic reload is REQUIRED:

- The software MUST detect `WORKFLOW.md` changes.
- On change, it MUST re-read and re-apply workflow config and prompt template without restart.
- The software MUST attempt to adjust live behavior to the new config (for example polling
  cadence, concurrency limits, active/terminal states, CLI settings, workspace paths/hooks, and
  prompt content for future runs).
- Reloaded config applies to future dispatch, retry scheduling, reconciliation decisions, hook
  execution, and agent launches.
- Implementations are not REQUIRED to restart in-flight agent sessions automatically when config
  changes.
- Changes that swap the tracker adapter kind or the CLI adapter kind MAY require restart and SHOULD
  be documented as such; in-flight sessions started under the previous adapter SHOULD be allowed to
  finish.
- Extensions that manage their own listeners/resources (for example an HTTP server port change) MAY
  require restart unless the implementation explicitly supports live rebind.
- Implementations SHOULD also re-validate/reload defensively during runtime operations (for example
  before dispatch) in case filesystem watch events are missed.
- Invalid reloads MUST NOT crash the service; keep operating with the last known good effective
  configuration and emit an operator-visible error.

### 6.3 Dispatch Preflight Validation

This validation is a scheduler preflight run before attempting to dispatch new work. It validates
the workflow/config needed to poll and launch workers, not a full audit of all possible workflow
behavior.

Startup validation:

- Validate configuration before starting the scheduling loop.
- If startup validation fails, fail startup and emit an operator-visible error.

Per-tick dispatch validation:

- Re-validate before each dispatch cycle.
- If validation fails, skip dispatch for that tick, keep reconciliation active, and emit an
  operator-visible error.

Validation checks (core):

- Workflow file can be loaded and parsed.
- `tracker.kind` is present and matches a known tracker adapter.
- `cli.command` is present and non-empty.
- A CLI adapter resolves from `cli.command`.

Validation checks (delegated to adapters):

- Tracker-specific validation (auth presence, project identity, etc.) is delegated to the tracker
  adapter's `validate` operation (Section 11.1).
- CLI-specific validation (binary discoverable on PATH, options well-formed, etc.) is delegated to
  the CLI adapter's `validate` operation (Section 10.1).
- An adapter validation failure is treated as a dispatch validation failure.

### 6.4 Core Config Fields Summary (Cheat Sheet)

This section is intentionally redundant so a coding agent can implement the config layer quickly.
Adapter-specific fields are documented in the adapter sections that define them. Core conformance
does not require recognizing or validating adapter fields beyond delegating to the adapter.

- `tracker.kind`: string, REQUIRED, reference value `github`, extension value `linear`
- `tracker.active_states`: list of strings, default RECOMMENDED `["Todo", "In Progress"]`
- `tracker.terminal_states`: list of strings, default RECOMMENDED `["Done", "Cancelled"]`
- (tracker-kind-specific fields: see Section 11.4 for GitHub, Appendix B for Linear)
- `polling.interval_ms`: integer, default `30000`
- `workspace.root`: path resolved to absolute, default `<system-temp>/sympheo_workspaces`
- `hooks.after_create`: shell script or null
- `hooks.before_run`: shell script or null
- `hooks.after_run`: shell script or null
- `hooks.before_remove`: shell script or null
- `hooks.timeout_ms`: integer, default `60000`
- `agent.max_concurrent_agents`: integer, default `10`
- `agent.max_turns`: integer, default `20`
- `agent.max_retry_backoff_ms`: integer, default `300000` (5m)
- `agent.max_concurrent_agents_by_state`: map of positive integers, default `{}`
- `cli.command`: shell command string, REQUIRED, OpenCode default `opencode run`
- `cli.args`: list of strings, default `[]`
- `cli.env`: map of strings, default `{}`
- `cli.turn_timeout_ms`: integer, default `3600000`
- `cli.read_timeout_ms`: integer, default `5000`
- `cli.stall_timeout_ms`: integer, default `300000`
- `cli.options`: map, default `{}` (adapter-specific)

## 7. Orchestration State Machine

The orchestrator is the only component that mutates scheduling state. All worker outcomes are
reported back to it and converted into explicit state transitions.

### 7.1 Issue Orchestration States

This is not the same as tracker states (`Todo`, `In Progress`, etc.). This is the service's
internal claim state.

1. `Unclaimed`
   - Issue is not running and has no retry scheduled.

2. `Claimed`
   - Orchestrator has reserved the issue to prevent duplicate dispatch.
   - In practice, claimed issues are either `Running` or `RetryQueued`.

3. `Running`
   - Worker task exists and the issue is tracked in `running` map.

4. `RetryQueued`
   - Worker is not running, but a retry timer exists in `retry_attempts`.

5. `Released`
   - Claim removed because issue is terminal, non-active, missing, or retry path completed without
     re-dispatch.

Important nuance:

- A successful worker exit does not mean the issue is done forever.
- The worker MAY continue through multiple back-to-back coding-agent turns before it exits.
- After each normal turn completion, the worker re-checks the tracker issue state.
- If the issue is still in an active state, the worker SHOULD start another turn on the same agent
  session (resumed via the CLI adapter's session-resume mechanism) in the same workspace, up to
  `agent.max_turns`.
- The first turn SHOULD use the full rendered task prompt.
- Continuation turns SHOULD send only continuation guidance to the resumed session, not resend the
  original task prompt that is already present in session history.
- Once the worker exits normally, the orchestrator still schedules a short continuation retry
  (about 1 second) so it can re-check whether the issue remains active and needs another worker
  session.

### 7.2 Run Attempt Lifecycle

A run attempt transitions through these phases:

1. `PreparingWorkspace`
2. `BuildingPrompt`
3. `LaunchingAgentTurn`
4. `InitializingSession` (first turn only; subsequent turns resume)
5. `StreamingTurn`
6. `Finishing`
7. `Succeeded`
8. `Failed`
9. `TimedOut`
10. `Stalled`
11. `CanceledByReconciliation`

Distinct terminal reasons are important because retry logic and logs differ.

### 7.3 Transition Triggers

- `Poll Tick`
  - Reconcile active runs.
  - Validate config.
  - Fetch candidate issues.
  - Dispatch until slots are exhausted.

- `Worker Exit (normal)`
  - Remove running entry.
  - Update aggregate runtime totals.
  - Schedule continuation retry (attempt `1`) after the worker exhausts or finishes its in-process
    turn loop.

- `Worker Exit (abnormal)`
  - Remove running entry.
  - Update aggregate runtime totals.
  - Schedule exponential-backoff retry.

- `Agent Update Event` (parsed by the CLI adapter from CLI stdout/logs)
  - Update live session fields, token counters, and rate limits.

- `Retry Timer Fired`
  - Re-fetch active candidates and attempt re-dispatch, or release claim if no longer eligible.

- `Reconciliation State Refresh`
  - Stop runs whose issue states are terminal or no longer active.

- `Stall Timeout`
  - Kill worker and schedule retry.

### 7.4 Idempotency and Recovery Rules

- The orchestrator serializes state mutations through one authority to avoid duplicate dispatch.
- `claimed` and `running` checks are REQUIRED before launching any worker.
- Reconciliation runs before dispatch on every tick.
- Restart recovery is tracker-driven and filesystem-driven (without a durable orchestrator DB).
- Startup terminal cleanup removes stale workspaces for issues already in terminal states.

## 8. Polling, Scheduling, and Reconciliation

### 8.1 Poll Loop

At startup, the service validates config, performs startup cleanup, schedules an immediate tick,
and then repeats every `polling.interval_ms`.

The effective poll interval SHOULD be updated when workflow config changes are re-applied.

Tick sequence:

1. Reconcile running issues.
2. Run dispatch preflight validation.
3. Fetch candidate issues from tracker using active states.
4. Sort issues by dispatch priority.
5. Dispatch eligible issues while slots remain.
6. Notify observability/status consumers of state changes.

If per-tick validation fails, dispatch is skipped for that tick, but reconciliation still happens
first.

### 8.2 Candidate Selection Rules

An issue is dispatch-eligible only if all are true:

- It has `id`, `identifier`, `title`, and `state`.
- Its state is in `active_states` and not in `terminal_states`.
- It is not already in `running`.
- It is not already in `claimed`.
- Global concurrency slots are available.
- Per-state concurrency slots are available.
- Blocker rule for the first active state passes:
  - The first entry in `active_states` is treated as the "fresh work" state (typically `Todo`).
  - When the issue is in that state, do not dispatch when any blocker is non-terminal.
  - Issues already in later active states (typically `In Progress`) skip the blocker gate, since
    work has already begun.

Sorting order (stable intent):

1. `priority` ascending (1..4 are preferred; null/unknown sorts last)
2. `created_at` oldest first
3. `identifier` lexicographic tie-breaker

### 8.3 Concurrency Control

Global limit:

- `available_slots = max(max_concurrent_agents - running_count, 0)`

Per-state limit:

- `max_concurrent_agents_by_state[state]` if present (state key normalized)
- otherwise fallback to global limit

The runtime counts issues by their current tracked state in the `running` map.

### 8.4 Retry and Backoff

Retry entry creation:

- Cancel any existing retry timer for the same issue.
- Store `attempt`, `identifier`, `error`, `due_at_ms`, and new timer handle.

Backoff formula:

- Normal continuation retries after a clean worker exit use a short fixed delay of `1000` ms.
- Failure-driven retries use `delay = min(10000 * 2^(attempt - 1), agent.max_retry_backoff_ms)`.
- Power is capped by the configured max retry backoff (default `300000` / 5m).

Retry handling behavior:

1. Fetch active candidate issues (not all issues).
2. Find the specific issue by `issue_id`.
3. If not found, release claim.
4. If found and still candidate-eligible:
   - Dispatch if slots are available.
   - Otherwise requeue with error `no available orchestrator slots`.
5. If found but no longer active, release claim.

Note:

- Terminal-state workspace cleanup is handled by startup cleanup and active-run reconciliation
  (including terminal transitions for currently running issues).
- Retry handling mainly operates on active candidates and releases claims when the issue is absent,
  rather than performing terminal cleanup itself.

### 8.5 Active Run Reconciliation

Reconciliation runs every tick and has two parts.

Part A: Stall detection

- For each running issue, compute `elapsed_ms` since:
  - `last_agent_timestamp` if any event has been seen, else
  - `started_at`
- If `elapsed_ms > cli.stall_timeout_ms`, terminate the worker and queue a retry.
- If `stall_timeout_ms <= 0`, skip stall detection entirely.

Part B: Tracker state refresh

- Fetch current issue states for all running issue IDs.
- For each running issue:
  - If tracker state is terminal: terminate worker and clean workspace.
  - If tracker state is still active: update the in-memory issue snapshot.
  - If tracker state is neither active nor terminal: terminate worker without workspace cleanup.
- If state refresh fails, keep workers running and try again on the next tick.

### 8.6 Startup Terminal Workspace Cleanup

When the service starts:

1. Query tracker for issues in terminal states.
2. For each returned issue identifier, remove the corresponding workspace directory.
3. If the terminal-issues fetch fails, log a warning and continue startup.

This prevents stale terminal workspaces from accumulating after restarts.

## 9. Workspace Management and Safety

### 9.1 Workspace Layout

Workspace root:

- `workspace.root` (normalized absolute path)

Per-issue workspace path:

- `<workspace.root>/<sanitized_issue_identifier>`

Workspace persistence:

- Workspaces are reused across runs for the same issue.
- Successful runs do not auto-delete workspaces.

### 9.2 Workspace Creation and Reuse

Input: `issue.identifier`

Algorithm summary:

1. Sanitize identifier to `workspace_key` (Section 4.2).
2. Compute workspace path under workspace root.
3. Ensure the workspace path exists as a directory.
4. Mark `created_now=true` only if the directory was created during this call; otherwise
   `created_now=false`.
5. If `created_now=true`, run `after_create` hook if configured.

Notes:

- This section does not assume any specific repository/VCS workflow.
- Workspace preparation beyond directory creation (for example dependency bootstrap, checkout/sync,
  code generation) is implementation-defined and is typically handled via hooks.

### 9.3 OPTIONAL Workspace Population (Implementation-Defined)

The spec does not require any built-in VCS or repository bootstrap behavior.

Implementations MAY populate or synchronize the workspace using implementation-defined logic
and/or hooks (for example `after_create` and/or `before_run`).

Failure handling:

- Workspace population/synchronization failures return an error for the current attempt.
- If failure happens while creating a brand-new workspace, implementations MAY remove the partially
  prepared directory.
- Reused workspaces SHOULD NOT be destructively reset on population failure unless that policy is
  explicitly chosen and documented.

### 9.4 Workspace Hooks

Supported hooks:

- `hooks.after_create`
- `hooks.before_run`
- `hooks.after_run`
- `hooks.before_remove`

Execution contract:

- Execute in a local shell context appropriate to the host OS, with the workspace directory as
  `cwd`.
- On POSIX systems, `sh -lc <script>` (or a stricter equivalent such as `bash -lc <script>`) is a
  conforming default.
- Hook timeout uses `hooks.timeout_ms`; default: `60000 ms`.
- Log hook start, failures, and timeouts.
- The `issue.identifier`, `issue.id`, and workspace path SHOULD be exposed to hooks via environment
  variables (RECOMMENDED: `SYMPHEO_ISSUE_IDENTIFIER`, `SYMPHEO_ISSUE_ID`, `SYMPHEO_WORKSPACE_PATH`).

Failure semantics:

- `after_create` failure or timeout is fatal to workspace creation.
- `before_run` failure or timeout is fatal to the current run attempt.
- `after_run` failure or timeout is logged and ignored.
- `before_remove` failure or timeout is logged and ignored.

### 9.5 Safety Invariants

This is the most important portability constraint.

Invariant 1: Run the coding-agent CLI only in the per-issue workspace path.

- Before launching the CLI subprocess, validate:
  - `cwd == workspace_path`

Invariant 2: Workspace path MUST stay inside workspace root.

- Normalize both paths to absolute.
- Require `workspace_path` to have `workspace_root` as a prefix directory.
- Reject any path outside the workspace root.

Invariant 3: Workspace key is sanitized.

- Only `[A-Za-z0-9._]` allowed in workspace directory names.
- Replace all other characters with `-`.

## 10. CLI Adapter Contract

This section defines the language-neutral contract Sympheo expects from a coding-agent CLI adapter.
The reference adapter targets **OpenCode** (`opencode run`); other adapters (for example `pi.dev`)
MUST satisfy the same contract.

The orchestrator never speaks directly to a specific CLI binary; all CLI-specific behavior is
encapsulated by the adapter.

### 10.1 Adapter Identity and Selection

Each CLI adapter declares:

- A `kind` string (for example `opencode`, `pidev`).
- A list of binary names it claims to handle (for example `opencode`).
- A `validate(cli_config)` operation that performs static configuration checks (binary present on
  PATH, options well-formed, REQUIRED `cli.options` keys provided, etc.).

Selection rule:

- The orchestrator inspects the leading binary token of `cli.command` and selects the first adapter
  whose claimed binaries match.
- If no adapter matches, dispatch validation fails with `cli_adapter_not_found`.

### 10.2 Lifecycle Operations

A CLI adapter MUST implement the following operations. Each operation is invoked by the worker
within a single run attempt; the orchestrator never invokes the adapter concurrently for the same
worker.

#### 10.2.1 `start_session(workspace_path, cli_config) -> SessionContext | error`

- Performs any one-time setup needed for the worker run.
- Does NOT necessarily start a CLI subprocess. For `opencode run`, this typically allocates an
  opaque session handle (UUID or CLI-managed identifier) that will be passed via `--session` on
  each turn.
- Returns a `SessionContext` containing at minimum:
  - `agent_session_handle` (string)
  - `session_id` (string, opaque, unique per worker run)
- Errors map to normalized categories (Section 10.5).

#### 10.2.2 `run_turn(session_context, prompt, issue, turn_number, on_event) -> TurnResult | error`

- Launches one CLI subprocess invocation for one turn.
- Working directory MUST be the per-issue workspace path.
- Subprocess invocation: `bash -lc "<cli.command> <adapter-injected-args> <cli.args>"`.
- The adapter MAY inject session-resume arguments (for OpenCode: `--session <handle>` after the
  first turn) and prompt-input arguments according to the targeted CLI's contract.
- The adapter parses CLI stdout and/or log files to extract structured events and forwards them via
  the `on_event` callback.
- Enforces `cli.turn_timeout_ms` and `cli.read_timeout_ms`.
- Returns a `TurnResult` containing at minimum:
  - `outcome` (enum: `succeeded`, `failed`, `cancelled`, `timed_out`)
  - `usage` (OPTIONAL token usage delta or absolute totals)
  - `last_message` (OPTIONAL summarized final assistant message)
  - `error` (OPTIONAL normalized error)

Continuation semantics:

- The first call to `run_turn` for a session uses the full rendered task prompt.
- Subsequent calls within the same worker run use continuation guidance (a short prompt directing
  the agent to continue work on the existing session).
- The adapter is responsible for ensuring the CLI invocation resumes the existing session rather
  than starting a fresh one.

#### 10.2.3 `stop_session(session_context) -> void`

- Performs any final teardown the adapter needs.
- For `opencode run`, this is typically a no-op since each turn is a separate process.
- MUST be safe to call after a `run_turn` failure.

### 10.3 Event Parsing and Normalization

The adapter parses CLI output and emits normalized events to the orchestrator. Each event SHOULD
include:

- `event` (enum/string)
- `timestamp` (UTC timestamp)
- `agent_pid` (if available, PID of the active turn subprocess)
- OPTIONAL `usage` map (token counts; absolute or delta, see Section 13.5)
- OPTIONAL `rate_limits` map
- payload fields as needed

RECOMMENDED normalized event names:

- `session_started`
- `startup_failed`
- `turn_started`
- `turn_message` (intermediate assistant or tool output)
- `turn_completed`
- `turn_failed`
- `turn_cancelled`
- `turn_timed_out`
- `tool_call`
- `tool_result`
- `notification`
- `usage_update`
- `rate_limit_update`
- `other_message`
- `malformed`

Output sources MAY include:

- CLI stdout (structured JSON lines if the CLI supports them, otherwise best-effort text parsing).
- CLI stderr (typically diagnostics; the adapter SHOULD keep diagnostic stderr separate from event
  parsing unless the CLI emits structured events on stderr).
- CLI log files (some CLIs persist a structured log file per session; adapters MAY tail this file).

The adapter MUST document its parsing strategy and the CLI version range it supports.

### 10.4 Approval, Tool Calls, and User Input Policy

Approval, sandbox, and user-input behavior is implementation-defined and adapter-specific.

Policy requirements:

- Each implementation MUST document its chosen approval, sandbox, and operator-confirmation
  posture.
- Approval requests and user-input-required events MUST NOT leave a run stalled indefinitely. An
  adapter MAY either configure the CLI to auto-approve, surface the request to an operator,
  auto-resolve it, or fail the run according to its documented policy.

Example high-trust behavior for the OpenCode adapter:

- Configure `opencode run` with permissive permissions (no interactive approval prompts).
- Treat any unexpected interactive prompt as a hard failure of the turn.

Unsupported tool invocations:

- If the CLI requests a tool that is not configured or supported, the adapter SHOULD ensure the CLI
  receives a failure response and continues, rather than hanging the run.

### 10.5 Error Mapping

RECOMMENDED normalized error categories returned by adapter operations:

- `cli_not_found`
- `invalid_workspace_cwd`
- `session_start_failed`
- `turn_launch_failed`
- `turn_read_timeout`
- `turn_total_timeout`
- `turn_cancelled`
- `turn_failed`
- `subprocess_exit`
- `output_parse_error`
- `user_input_required`

### 10.6 OpenCode Reference Adapter

This subsection documents the reference adapter behavior for `cli.kind` = OpenCode (selected when
the leading binary of `cli.command` is `opencode`).

Defaults:

- `cli.command` default: `opencode run`

Session model:

- `start_session` allocates an opaque session handle (RECOMMENDED: a UUIDv4 generated by Sympheo
  that is passed to OpenCode via `--session`, or by reading the session ID OpenCode prints on first
  invocation, depending on the targeted OpenCode version).
- `run_turn` invokes `opencode run --session <handle> <cli.args> -- <prompt>` (exact flag names are
  resolved against the targeted OpenCode version).
- The first turn sends the full rendered prompt. Continuation turns send a short continuation
  message such as `Continue working on the issue.`.

Output parsing:

- The adapter parses OpenCode stdout/log output to extract:
  - the final assistant message (`turn_completed` event with `last_message`)
  - tool invocations (`tool_call` / `tool_result` events)
  - token usage (input/output/total) when reported
  - any rate-limit information when reported
- If OpenCode exposes a structured JSON log mode, the adapter SHOULD prefer it over scraping
  free-form output.
- The adapter MUST document the OpenCode version range it has been tested against.

`cli.options` recognized by the OpenCode adapter (illustrative, exact set is adapter-defined):

- `options.model` (string)
- `options.permissions` (map or list, mapped to OpenCode permission flags)
- `options.mcp_servers` (list, mapped to OpenCode MCP server configuration)

The adapter MUST ignore unknown `cli.options` keys for forward compatibility, but SHOULD log a
warning so operators can detect typos.

### 10.7 Worker Algorithm (CLI-Agnostic)

The worker uses the CLI adapter as follows:

1. Create/reuse workspace for issue.
2. Run `before_run` hook.
3. `session = adapter.start_session(workspace_path, cli_config)`.
4. For each turn from 1 to `agent.max_turns`:
   a. Build prompt (full task prompt on turn 1, continuation guidance on subsequent turns).
   b. `result = adapter.run_turn(session, prompt, issue, turn, on_event)`.
   c. If `result.outcome != succeeded`, exit the loop and fail the worker.
   d. Refresh the issue state via the tracker adapter.
   e. If the issue is no longer in an active state, exit the loop normally.
5. `adapter.stop_session(session)`.
6. Run `after_run` hook (best effort).
7. Exit normally.

## 11. Tracker Adapter Contract

This section defines the language-neutral contract Sympheo expects from a tracker adapter.
The reference adapter targets **GitHub Projects (v2)**; an extension adapter for **Linear** is
documented in Appendix B.

The orchestrator never speaks directly to a specific tracker API; all tracker-specific behavior
is encapsulated by the adapter.

### 11.1 REQUIRED Operations

A tracker adapter MUST implement:

#### 11.1.1 `validate(tracker_config) -> ok | error`

- Performs static configuration checks before dispatch:
  - Required fields present (auth, project identity, etc.).
  - Auth token resolves to a non-empty value after `$VAR` indirection.
- Does NOT perform network calls.

#### 11.1.2 `fetch_candidate_issues() -> [Issue] | error`

- Returns issues currently in any of the configured `tracker.active_states`.
- Pagination is handled internally; the caller receives the full list.
- Issues MUST be normalized to the domain model in Section 4.1.1.

#### 11.1.3 `fetch_issues_by_states(state_names) -> [Issue] | error`

- Returns issues currently in any of the supplied state names.
- Used for startup terminal cleanup.
- An empty input list MUST return an empty result without making API calls.

#### 11.1.4 `fetch_issue_states_by_ids(issue_ids) -> [Issue] | error`

- Returns minimal normalized issue records (at least `id`, `identifier`, `state`) for the supplied
  IDs.
- Used for active-run reconciliation.
- Missing IDs (issues that no longer exist) SHOULD be omitted from the result rather than producing
  an error.

### 11.2 Normalization Requirements

All adapters MUST produce issues conforming to the domain model in Section 4.1.1. Specifically:

- `labels` are lowercased.
- `priority` is an integer or `null`. Non-integer values map to `null`.
- `created_at` and `updated_at` are ISO-8601 timestamps or `null`.
- `blocked_by` is a list (possibly empty) of blocker refs.
- `state` is the raw state name from the tracker. State comparison is performed by the orchestrator
  after `lowercase` normalization (Section 4.2).

### 11.3 Error Categories

RECOMMENDED normalized error categories returned by adapter operations:

- `unsupported_tracker_kind`
- `missing_tracker_auth`
- `missing_tracker_project_identity`
- `tracker_request_failed` (transport failures)
- `tracker_status_error` (non-200 HTTP)
- `tracker_graphql_errors`
- `tracker_unknown_payload`
- `tracker_pagination_error`

Orchestrator behavior on tracker errors:

- Candidate fetch failure: log and skip dispatch for this tick.
- Running-state refresh failure: log and keep active workers running.
- Startup terminal cleanup failure: log warning and continue startup.

### 11.4 GitHub Reference Adapter

This subsection defines the reference adapter behavior for `tracker.kind = github`.

#### 11.4.1 `tracker` Front Matter Fields (GitHub)

- `kind` (string)
  - REQUIRED. Value: `github`.
- `org` (string)
  - REQUIRED. The GitHub organization that owns the project.
- `project_number` (integer)
  - REQUIRED. The GitHub Project (v2) number under that org.
- `status_field` (string)
  - REQUIRED. Name of the ProjectV2 single-select field used to track issue status (typically
    `Status`).
- `priority_field` (string, OPTIONAL)
  - Name of a ProjectV2 field used to derive `issue.priority`.
  - The field MAY be a single-select (option name parsed as integer when possible) or a number
    field.
  - When omitted, `issue.priority` is `null` for all issues.
- `endpoint` (string, OPTIONAL)
  - Default: `https://api.github.com/graphql`.
  - Permits targeting GitHub Enterprise Server.
- `auth_token` (string, OPTIONAL)
  - Default: `$SYMPHEO_GITHUB_TOKEN`.
  - MAY be a literal token or `$VAR_NAME`. If `$VAR_NAME` resolves to an empty string, treat as
    missing.
- `active_states` (list of strings)
  - Default: `["Todo", "In Progress"]` (matches the GitHub-default Status field options).
- `terminal_states` (list of strings)
  - Default: `["Done", "Cancelled"]`.

#### 11.4.2 Identifier Format

`issue.identifier` is `<repo>#<number>` where `<repo>` is the GitHub repository name (no owner
prefix) and `<number>` is the issue number. The owner is implied by the project's org and is not
embedded in the identifier.

Example: an issue numbered 42 in repo `sympheo` owned by org `supergeoff` produces
`identifier = "sympheo#42"` and `workspace_key = "sympheo-42"`.

#### 11.4.3 Branch Name

`issue.branch_name` is generated as:

- `<number>-<slugified-title>`
- `slugified-title`: lowercase the title, replace runs of non-`[a-z0-9]` characters with `-`, trim
  leading/trailing `-`, truncate to 60 characters.

Example: title `Fix login bug on Safari`, number `42` -> `branch_name = "42-fix-login-bug-on-safari"`.

#### 11.4.4 Status Field Semantics

- The configured `status_field` MUST exist on the project as a single-select field; otherwise
  validation fails.
- `issue.state` is the option name of the issue's value for that field.
- An issue that exists on the project but has no value for the status field is treated as having
  state equal to the empty string `""`, which never matches active or terminal states (so it is
  ignored by dispatch and reconciliation).

#### 11.4.5 Project Membership

- The adapter only considers issues that are items of the configured project (`ProjectV2Item`
  entries pointing to issues).
- Pull requests, draft items, or items without an associated issue are ignored.
- The repo of the underlying issue is preserved as the source of `<repo>` in `identifier`. Issues
  from any repo referenced by the project are eligible (no per-repo filtering at the adapter
  level).

#### 11.4.6 Blockers via GitHub Issue Dependencies

- The adapter uses GitHub's native Issue Dependencies feature
  (`trackedInIssues` / `trackedIssues` GraphQL fields on issues).
- For each candidate issue, blockers are derived from `trackedInIssues` (the issues that the
  candidate depends on).
- Each blocker's `state` is determined by looking up its presence on the same project and reading
  its `status_field` value (best effort; if the blocker is not on the project, `state = null`).
- If GitHub Issue Dependencies are not enabled or supported on the target org/repo, the adapter
  MUST fall back to parsing `Blocked by #N` references in the issue body, and MAY also recognize
  `Depends on #N`. The fallback MUST be documented and logged once at startup.

#### 11.4.7 Auth and HTTP

- HTTP requests are sent with `Authorization: Bearer <token>` and `Accept: application/vnd.github+json`.
- Default request timeout: `30000 ms`.
- Pagination uses GitHub's `pageInfo.endCursor` / `hasNextPage` pattern with a default page size
  of `50`.

#### 11.4.8 Validation

- `tracker.kind == "github"`.
- `tracker.org` is non-empty.
- `tracker.project_number` is a positive integer.
- `tracker.status_field` is non-empty.
- `tracker.auth_token` resolves to a non-empty string.

#### 11.4.9 GraphQL Notes

- GitHub's GraphQL schema for ProjectV2 changes from time to time. Keep all GraphQL query
  construction isolated in the adapter.
- The adapter MUST request only the fields it normalizes; over-broad queries SHOULD be avoided to
  reduce token usage and rate-limit pressure.

### 11.5 Tracker Writes (Important Boundary)

Sympheo does not require first-class tracker write APIs in the orchestrator.

- Ticket mutations (state transitions, comments, PR metadata) are typically handled by the coding
  agent using tools defined by the workflow prompt (for example MCP servers, the `gh` CLI inside
  the workspace, or the OPTIONAL `github_graphql` client-side tool extension below).
- The service remains a scheduler/runner and tracker reader.
- Workflow-specific success often means "reached the next handoff state" (for example
  `Human Review`) rather than tracker terminal state `Done`.
- Sympheo does not validate agent-claimed outcomes. It does not check that PRs were opened, that
  artifacts exist, or that tests passed before the agent transitions a ticket. Such guarantees are
  the responsibility of the workflow prompt, agent tooling, and downstream review.

### 11.6 OPTIONAL `github_graphql` Tool Extension

This is an OPTIONAL client-side tool extension that an adapter or runtime MAY expose to the coding
agent so the agent can issue GitHub GraphQL queries and mutations using Sympheo's configured
tracker auth.

Whether and how this tool is surfaced to the agent depends on the CLI adapter (for example via an
MCP server, a built-in CLI tool registration mechanism, or workspace-level scripts). The contract
below applies regardless of the surfacing mechanism.

Contract:

- Purpose: execute a raw GraphQL query or mutation against GitHub using Sympheo's configured
  tracker auth for the current session.
- Availability: only meaningful when `tracker.kind = "github"` and valid GitHub auth is configured.
- Endpoint: the tracker's configured `endpoint` (default `https://api.github.com/graphql`).
- Preferred input shape:

  ```json
  {
    "query": "single GraphQL query or mutation document",
    "variables": {
      "optional": "graphql variables object"
    }
  }
  ```

- `query` MUST be a non-empty string.
- `query` MUST contain exactly one GraphQL operation.
- `variables` is OPTIONAL and, when present, MUST be a JSON object.
- Implementations MAY additionally accept a raw GraphQL query string as shorthand input.
- Execute one GraphQL operation per tool call.
- If the provided document contains multiple operations, reject the tool call as invalid input.
- `operationName` selection is intentionally out of scope for this extension.
- Reuse the configured GitHub endpoint and auth from the active Sympheo workflow/runtime config;
  do not require the coding agent to read raw tokens from disk.
- No additional scope restriction is imposed by this extension; the available scope matches the
  scopes granted to `SYMPHEO_GITHUB_TOKEN`.
- Tool result semantics:
  - transport success + no top-level GraphQL `errors` -> `success=true`
  - top-level GraphQL `errors` present -> `success=false`, but preserve the GraphQL response body
    for debugging
  - invalid input, missing auth, or transport failure -> `success=false` with an error payload
- Return the GraphQL response or error payload as structured tool output that the model can inspect
  in-session.

## 12. Prompt Construction and Context Assembly

### 12.1 Inputs

Inputs to prompt rendering:

- `workflow.prompt_template`
- normalized `issue` object
- OPTIONAL `attempt` integer (retry/continuation metadata)

### 12.2 Rendering Rules

- Render with strict variable checking.
- Render with strict filter checking.
- Convert issue object keys to strings for template compatibility.
- Preserve nested arrays/maps (labels, blockers) so templates can iterate.

### 12.3 Retry/Continuation Semantics

`attempt` SHOULD be passed to the template because the workflow prompt can provide different
instructions for:

- first run (`attempt` null or absent)
- continuation run after a successful prior session
- retry after error/timeout/stall

### 12.4 Failure Semantics

If prompt rendering fails:

- Fail the run attempt immediately.
- Let the orchestrator treat it like any other worker failure and decide retry behavior.

## 13. Logging, Status, and Observability

### 13.1 Logging Conventions

REQUIRED context fields for issue-related logs:

- `issue_id`
- `issue_identifier`

REQUIRED context for coding-agent session lifecycle logs:

- `session_id`

Message formatting requirements:

- Use stable `key=value` phrasing.
- Include action outcome (`completed`, `failed`, `retrying`, etc.).
- Include concise failure reason when present.
- Avoid logging large raw payloads unless necessary.

### 13.2 Logging Outputs and Sinks

The spec does not prescribe where logs are written (stderr, file, remote sink, etc.).

Requirements:

- Operators MUST be able to see startup/validation/dispatch failures without attaching a debugger.
- Implementations MAY write to one or more sinks.
- If a configured log sink fails, the service SHOULD continue running when possible and emit an
  operator-visible warning through any remaining sink.

### 13.3 Runtime Snapshot / Monitoring Interface (OPTIONAL but RECOMMENDED)

If the implementation exposes a synchronous runtime snapshot (for dashboards or monitoring), it
SHOULD return:

- `running` (list of running session rows)
- each running row SHOULD include `turn_count`
- `retrying` (list of retry queue rows)
- `agent_totals`
  - `input_tokens`
  - `output_tokens`
  - `total_tokens`
  - `seconds_running` (aggregate runtime seconds as of snapshot time, including active sessions)
- `rate_limits` (latest agent rate limit payload, if available)

RECOMMENDED snapshot error modes:

- `timeout`
- `unavailable`

### 13.4 OPTIONAL Human-Readable Status Surface

A human-readable status surface (terminal output, dashboard, etc.) is OPTIONAL and
implementation-defined.

If present, it SHOULD draw from orchestrator state/metrics only and MUST NOT be REQUIRED for
correctness.

### 13.5 Session Metrics and Token Accounting

Token accounting rules:

- Agent events emitted by the CLI adapter can include token counts in multiple payload shapes
  depending on the underlying CLI.
- Each adapter MUST document whether the token counts it emits are absolute thread totals or
  per-turn deltas.
- Sympheo prefers absolute totals when available and tracks deltas relative to last reported
  totals to avoid double-counting.
- For per-turn deltas, Sympheo accumulates them into the running entry and the global totals.
- Do not treat generic `usage` maps as cumulative totals unless the adapter declares them as such.
- Accumulate aggregate totals in orchestrator state.

Runtime accounting:

- Runtime SHOULD be reported as a live aggregate at snapshot/render time.
- Implementations MAY maintain a cumulative counter for ended sessions and add active-session
  elapsed time derived from `running` entries (for example `started_at`) when producing a
  snapshot/status view.
- Add run duration seconds to the cumulative ended-session runtime when a session ends (normal exit
  or cancellation/termination).
- Continuous background ticking of runtime totals is not REQUIRED.

Rate-limit tracking:

- Track the latest rate-limit payload seen in any agent update (when the adapter exposes one).
- Any human-readable presentation of rate-limit data is implementation-defined.

### 13.6 Humanized Agent Event Summaries (OPTIONAL)

Humanized summaries of raw agent events are OPTIONAL.

If implemented:

- Treat them as observability-only output.
- Do not make orchestrator logic depend on humanized strings.

### 13.7 OPTIONAL HTTP Server Extension

This section defines an OPTIONAL HTTP interface for observability and operational control.

If implemented:

- The HTTP server is an extension and is not REQUIRED for conformance.
- The implementation MAY serve server-rendered HTML or a client-side application for the dashboard.
- The dashboard/API MUST be observability/control surfaces only and MUST NOT become REQUIRED for
  orchestrator correctness.

Extension config:

- `server.port` (integer, OPTIONAL)
  - Enables the HTTP server extension.
  - `0` requests an ephemeral port for local development and tests.
  - CLI `--port` overrides `server.port` when both are present.

Enablement (extension):

- Start the HTTP server when a CLI `--port` argument is provided.
- Start the HTTP server when `server.port` is present in `WORKFLOW.md` front matter.
- The `server` top-level key is owned by this extension.
- Positive `server.port` values bind that port.
- Implementations SHOULD bind loopback by default (`127.0.0.1` or host equivalent) unless explicitly
  configured otherwise.
- Changes to HTTP listener settings (for example `server.port`) do not need to hot-rebind;
  restart-required behavior is conformant.

#### 13.7.1 Human-Readable Dashboard (`/`)

- Host a human-readable dashboard at `/`.
- The returned document SHOULD depict the current state of the system (for example active sessions,
  retry delays, token consumption, runtime totals, recent events, and health/error indicators).
- It is up to the implementation whether this is server-generated HTML or a client-side app that
  consumes the JSON API below.

#### 13.7.2 JSON REST API (`/api/v1/*`)

Provide a JSON REST API under `/api/v1/*` for current runtime state and operational debugging.

Minimum endpoints:

- `GET /api/v1/state`
  - Returns a summary view of the current system state (running sessions, retry queue/delays,
    aggregate token/runtime totals, latest rate limits, and any additional tracked summary fields).
  - Suggested response shape:

    ```json
    {
      "generated_at": "2026-02-24T20:15:30Z",
      "counts": {
        "running": 2,
        "retrying": 1
      },
      "running": [
        {
          "issue_id": "I_kwDOABCD12345",
          "issue_identifier": "sympheo#42",
          "state": "In Progress",
          "session_id": "opencode-9f3b...-7",
          "turn_count": 7,
          "last_event": "turn_completed",
          "last_message": "",
          "started_at": "2026-02-24T20:10:12Z",
          "last_event_at": "2026-02-24T20:14:59Z",
          "tokens": {
            "input_tokens": 1200,
            "output_tokens": 800,
            "total_tokens": 2000
          }
        }
      ],
      "retrying": [
        {
          "issue_id": "I_kwDOABCD67890",
          "issue_identifier": "sympheo#43",
          "attempt": 3,
          "due_at": "2026-02-24T20:16:00Z",
          "error": "no available orchestrator slots"
        }
      ],
      "agent_totals": {
        "input_tokens": 5000,
        "output_tokens": 2400,
        "total_tokens": 7400,
        "seconds_running": 1834.2
      },
      "rate_limits": null
    }
    ```

- `GET /api/v1/<issue_identifier>`
  - Returns issue-specific runtime/debug details for the identified issue.
  - The path segment `<issue_identifier>` MUST be URL-encoded; for GitHub identifiers containing
    `#`, this means `sympheo%2342`.
  - Suggested response shape:

    ```json
    {
      "issue_identifier": "sympheo#42",
      "issue_id": "I_kwDOABCD12345",
      "status": "running",
      "workspace": {
        "path": "/tmp/sympheo_workspaces/sympheo-42"
      },
      "attempts": {
        "restart_count": 1,
        "current_retry_attempt": 2
      },
      "running": {
        "session_id": "opencode-9f3b...-7",
        "turn_count": 7,
        "state": "In Progress",
        "started_at": "2026-02-24T20:10:12Z",
        "last_event": "notification",
        "last_message": "Working on tests",
        "last_event_at": "2026-02-24T20:14:59Z",
        "tokens": {
          "input_tokens": 1200,
          "output_tokens": 800,
          "total_tokens": 2000
        }
      },
      "retry": null,
      "logs": {
        "agent_session_logs": [
          {
            "label": "latest",
            "path": "/var/log/sympheo/agent/sympheo-42/latest.log",
            "url": null
          }
        ]
      },
      "recent_events": [
        {
          "at": "2026-02-24T20:14:59Z",
          "event": "notification",
          "message": "Working on tests"
        }
      ],
      "last_error": null,
      "tracked": {}
    }
    ```

  - If the issue is unknown to the current in-memory state, return `404` with an error response (for
    example `{"error":{"code":"issue_not_found","message":"..."}}`).

- `POST /api/v1/refresh`
  - Queues an immediate tracker poll + reconciliation cycle (best-effort trigger; implementations
    MAY coalesce repeated requests).
  - Suggested request body: empty body or `{}`.
  - Suggested response (`202 Accepted`) shape:

    ```json
    {
      "queued": true,
      "coalesced": false,
      "requested_at": "2026-02-24T20:15:30Z",
      "operations": ["poll", "reconcile"]
    }
    ```

API design notes:

- The JSON shapes above are the RECOMMENDED baseline for interoperability and debugging ergonomics.
- Implementations MAY add fields, but SHOULD avoid breaking existing fields within a version.
- Endpoints SHOULD be read-only except for operational triggers like `/refresh`.
- Unsupported methods on defined routes SHOULD return `405 Method Not Allowed`.
- API errors SHOULD use a JSON envelope such as `{"error":{"code":"...","message":"..."}}`.
- If the dashboard is a client-side app, it SHOULD consume this API rather than duplicating state
  logic.

## 14. Failure Model and Recovery Strategy

### 14.1 Failure Classes

1. `Workflow/Config Failures`
   - Missing `WORKFLOW.md`
   - Invalid YAML front matter
   - Unsupported tracker kind or missing tracker credentials/project identity
   - Missing or unrecognized CLI adapter

2. `Workspace Failures`
   - Workspace directory creation failure
   - Workspace population/synchronization failure (implementation-defined; can come from hooks)
   - Invalid workspace path configuration
   - Hook timeout/failure

3. `Agent Session Failures`
   - Adapter `start_session` failure
   - Turn launch failure
   - Turn read timeout / turn total timeout
   - Turn failed/cancelled
   - Subprocess exit
   - User input requested and handled as failure by the implementation's documented policy
   - Stalled session (no activity)

4. `Tracker Failures`
   - Adapter request transport errors
   - Non-200 status
   - GraphQL errors
   - malformed payloads
   - pagination integrity errors

5. `Observability Failures`
   - Snapshot timeout
   - Dashboard render errors
   - Log sink configuration failure

### 14.2 Recovery Behavior

- Dispatch validation failures:
  - Skip new dispatches.
  - Keep service alive.
  - Continue reconciliation where possible.

- Worker failures:
  - Convert to retries with exponential backoff.

- Tracker candidate-fetch failures:
  - Skip this tick.
  - Try again on next tick.

- Reconciliation state-refresh failures:
  - Keep current workers.
  - Retry on next tick.

- Dashboard/log failures:
  - Do not crash the orchestrator.

### 14.3 Partial State Recovery (Restart)

Current design is intentionally in-memory for scheduler state.
Restart recovery means the service can resume useful operation by polling tracker state and reusing
preserved workspaces. It does not mean retry timers, running sessions, or live worker state survive
process restart.

After restart:

- No retry timers are restored from prior process memory.
- No running sessions are assumed recoverable.
- Service recovers by:
  - startup terminal workspace cleanup
  - fresh polling of active issues
  - re-dispatching eligible work

### 14.4 Operator Intervention Points

Operators can control behavior by:

- Editing `WORKFLOW.md` (prompt and most runtime settings).
- `WORKFLOW.md` changes are detected and re-applied automatically without restart according to
  Section 6.2.
- Changing issue states in the tracker:
  - terminal state -> running session is stopped and workspace cleaned when reconciled
  - non-active state -> running session is stopped without cleanup
- Restarting the service for process recovery, deployment, or adapter swap (not as the normal path
  for applying workflow config changes).

## 15. Security and Operational Safety

### 15.1 Trust Boundary Assumption

Each implementation defines its own trust boundary.

Operational safety requirements:

- Implementations SHOULD state clearly whether they are intended for trusted environments, more
  restrictive environments, or both.
- Implementations SHOULD state clearly whether they rely on auto-approved actions, operator
  approvals, stricter sandboxing, or some combination of those controls.
- Workspace isolation and path validation are important baseline controls, but they are not a
  substitute for whatever approval and sandbox policy an implementation chooses.
- Sympheo does NOT validate agent-claimed outcomes. The trust boundary explicitly extends to the
  agent: if the agent transitions a ticket without producing the expected artifacts, Sympheo will
  not detect it. Workflows that need such guarantees MUST encode them in the agent prompt, in
  workflow hooks, or in downstream review processes.

### 15.2 Filesystem Safety Requirements

Mandatory:

- Workspace path MUST remain under configured workspace root.
- Coding-agent CLI cwd MUST be the per-issue workspace path for the current run.
- Workspace directory names MUST use sanitized identifiers.

RECOMMENDED additional hardening for ports:

- Run under a dedicated OS user.
- Restrict workspace root permissions.
- Mount workspace root on a dedicated volume if possible.

### 15.3 Secret Handling

- Support `$VAR` indirection in workflow config.
- Do not log API tokens or secret env values.
- Validate presence of secrets without printing them.

### 15.4 Hook Script Safety

Workspace hooks are arbitrary shell scripts from `WORKFLOW.md`.

Implications:

- Hooks are fully trusted configuration.
- Hooks run inside the workspace directory.
- Hook output SHOULD be truncated in logs.
- Hook timeouts are REQUIRED to avoid hanging the orchestrator.

### 15.5 Harness Hardening Guidance

Running coding-agent CLIs against repositories, issue trackers, and other inputs that can contain
sensitive data or externally-controlled content can be dangerous. A permissive deployment can lead
to data leaks, destructive mutations, or full machine compromise if the agent is induced to execute
harmful commands or use overly-powerful integrations.

Implementations SHOULD explicitly evaluate their own risk profile and harden the execution harness
where appropriate. This specification intentionally does not mandate a single hardening posture, but
implementations SHOULD NOT assume that tracker data, repository contents, prompt inputs, or tool
arguments are fully trustworthy just because they originate inside a normal workflow.

Possible hardening measures include:

- Tightening CLI-adapter approval and sandbox settings (`cli.options`) instead of running with a
  maximally permissive configuration.
- Adding external isolation layers such as OS/container/VM sandboxing, network restrictions, or
  separate credentials beyond what the CLI's built-in policy controls offer.
- Filtering which tracker issues, projects, repos, labels, or other tracker sources are eligible
  for dispatch so untrusted or out-of-scope tasks do not automatically reach the agent.
- Narrowing tool extensions (for example `github_graphql`) so they can only read or mutate data
  inside the intended scope, rather than exposing general tracker-wide access.
- Reducing the set of client-side tools, MCP servers, credentials, filesystem paths, and network
  destinations available to the agent to the minimum needed for the workflow.

The correct controls are deployment-specific, but implementations SHOULD document them clearly and
treat harness hardening as part of the core safety model rather than an optional afterthought.

## 16. Reference Algorithms (Language-Agnostic)

### 16.1 Service Startup

```text
function start_service():
  configure_logging()
  start_observability_outputs()
  start_workflow_watch(on_change=reload_and_reapply_workflow)

  state = {
    poll_interval_ms: get_config_poll_interval_ms(),
    max_concurrent_agents: get_config_max_concurrent_agents(),
    running: {},
    claimed: set(),
    retry_attempts: {},
    completed: set(),
    agent_totals: {input_tokens: 0, output_tokens: 0, total_tokens: 0, seconds_running: 0},
    agent_rate_limits: null
  }

  tracker_adapter = resolve_tracker_adapter(config.tracker.kind)
  cli_adapter = resolve_cli_adapter(config.cli.command)

  validation = validate_dispatch_config(tracker_adapter, cli_adapter)
  if validation is not ok:
    log_validation_error(validation)
    fail_startup(validation)

  startup_terminal_workspace_cleanup(tracker_adapter)
  schedule_tick(delay_ms=0)

  event_loop(state)
```

### 16.2 Poll-and-Dispatch Tick

```text
on_tick(state):
  state = reconcile_running_issues(state, tracker_adapter)

  validation = validate_dispatch_config(tracker_adapter, cli_adapter)
  if validation is not ok:
    log_validation_error(validation)
    notify_observers()
    schedule_tick(state.poll_interval_ms)
    return state

  issues = tracker_adapter.fetch_candidate_issues()
  if issues failed:
    log_tracker_error()
    notify_observers()
    schedule_tick(state.poll_interval_ms)
    return state

  for issue in sort_for_dispatch(issues):
    if no_available_slots(state):
      break

    if should_dispatch(issue, state):
      state = dispatch_issue(issue, state, attempt=null)

  notify_observers()
  schedule_tick(state.poll_interval_ms)
  return state
```

### 16.3 Reconcile Active Runs

```text
function reconcile_running_issues(state, tracker_adapter):
  state = reconcile_stalled_runs(state)

  running_ids = keys(state.running)
  if running_ids is empty:
    return state

  refreshed = tracker_adapter.fetch_issue_states_by_ids(running_ids)
  if refreshed failed:
    log_debug("keep workers running")
    return state

  for issue in refreshed:
    if issue.state in terminal_states:
      state = terminate_running_issue(state, issue.id, cleanup_workspace=true)
    else if issue.state in active_states:
      state.running[issue.id].issue = issue
    else:
      state = terminate_running_issue(state, issue.id, cleanup_workspace=false)

  return state
```

### 16.4 Dispatch One Issue

```text
function dispatch_issue(issue, state, attempt):
  worker = spawn_worker(
    fn -> run_agent_attempt(issue, attempt, parent_orchestrator_pid) end
  )

  if worker spawn failed:
    return schedule_retry(state, issue.id, next_attempt(attempt), {
      identifier: issue.identifier,
      error: "failed to spawn agent"
    })

  state.running[issue.id] = {
    worker_handle,
    monitor_handle,
    identifier: issue.identifier,
    issue,
    session_id: null,
    agent_session_handle: null,
    agent_pid: null,
    last_agent_message: null,
    last_agent_event: null,
    last_agent_timestamp: null,
    agent_input_tokens: 0,
    agent_output_tokens: 0,
    agent_total_tokens: 0,
    last_reported_input_tokens: 0,
    last_reported_output_tokens: 0,
    last_reported_total_tokens: 0,
    retry_attempt: normalize_attempt(attempt),
    started_at: now_utc()
  }

  state.claimed.add(issue.id)
  state.retry_attempts.remove(issue.id)
  return state
```

### 16.5 Worker Attempt (Workspace + Prompt + Agent CLI)

```text
function run_agent_attempt(issue, attempt, orchestrator_channel):
  workspace = workspace_manager.create_for_issue(issue.identifier)
  if workspace failed:
    fail_worker("workspace error")

  if run_hook("before_run", workspace.path) failed:
    fail_worker("before_run hook error")

  session = cli_adapter.start_session(workspace.path, config.cli)
  if session failed:
    run_hook_best_effort("after_run", workspace.path)
    fail_worker("agent session startup error")

  send(orchestrator_channel, {agent_update, issue.id, {
    event: "session_started",
    session_id: session.session_id,
    agent_session_handle: session.agent_session_handle,
    timestamp: now_utc()
  }})

  max_turns = config.agent.max_turns
  turn_number = 1

  while true:
    prompt = build_turn_prompt(workflow_template, issue, attempt, turn_number, max_turns)
    if prompt failed:
      cli_adapter.stop_session(session)
      run_hook_best_effort("after_run", workspace.path)
      fail_worker("prompt error")

    turn_result = cli_adapter.run_turn(
      session_context=session,
      prompt=prompt,
      issue=issue,
      turn_number=turn_number,
      on_event=(evt) -> send(orchestrator_channel, {agent_update, issue.id, evt})
    )

    if turn_result.outcome != "succeeded":
      cli_adapter.stop_session(session)
      run_hook_best_effort("after_run", workspace.path)
      fail_worker("agent turn " + turn_result.outcome)

    refreshed_issue = tracker_adapter.fetch_issue_states_by_ids([issue.id])
    if refreshed_issue failed:
      cli_adapter.stop_session(session)
      run_hook_best_effort("after_run", workspace.path)
      fail_worker("issue state refresh error")

    issue = refreshed_issue[0] or issue

    if issue.state is not active:
      break

    if turn_number >= max_turns:
      break

    turn_number = turn_number + 1

  cli_adapter.stop_session(session)
  run_hook_best_effort("after_run", workspace.path)

  exit_normal()
```

### 16.6 Worker Exit and Retry Handling

```text
on_worker_exit(issue_id, reason, state):
  running_entry = state.running.remove(issue_id)
  state = add_runtime_seconds_to_totals(state, running_entry)

  if reason == normal:
    state.completed.add(issue_id)  # bookkeeping only
    state = schedule_retry(state, issue_id, 1, {
      identifier: running_entry.identifier,
      delay_type: continuation
    })
  else:
    state = schedule_retry(state, issue_id, next_attempt_from(running_entry), {
      identifier: running_entry.identifier,
      error: format("worker exited: %reason")
    })

  notify_observers()
  return state
```

```text
on_retry_timer(issue_id, state):
  retry_entry = state.retry_attempts.pop(issue_id)
  if missing:
    return state

  candidates = tracker_adapter.fetch_candidate_issues()
  if fetch failed:
    return schedule_retry(state, issue_id, retry_entry.attempt + 1, {
      identifier: retry_entry.identifier,
      error: "retry poll failed"
    })

  issue = find_by_id(candidates, issue_id)
  if issue is null:
    state.claimed.remove(issue_id)
    return state

  if available_slots(state) == 0:
    return schedule_retry(state, issue_id, retry_entry.attempt + 1, {
      identifier: issue.identifier,
      error: "no available orchestrator slots"
    })

  return dispatch_issue(issue, state, attempt=retry_entry.attempt)
```

## 17. Test and Validation Matrix

A conforming implementation SHOULD include tests that cover the behaviors defined in this
specification.

Validation profiles:

- `Core Conformance`: deterministic tests REQUIRED for all conforming implementations.
- `Adapter Conformance`: REQUIRED for each tracker or CLI adapter that the implementation ships.
- `Extension Conformance`: REQUIRED only for OPTIONAL features (HTTP server, `github_graphql` tool,
  Linear adapter, etc.) that an implementation chooses to ship.
- `Real Integration Profile`: environment-dependent smoke/integration checks RECOMMENDED before
  production use.

Unless otherwise noted, Sections 17.1 through 17.7 are `Core Conformance`. Bullets that begin with
`If ... is implemented` are `Extension Conformance`.

### 17.1 Workflow and Config Parsing

- Workflow file path precedence:
  - explicit runtime path is used when provided
  - cwd default is `WORKFLOW.md` when no explicit runtime path is provided
- Workflow file changes are detected and trigger re-read/re-apply without restart
- Invalid workflow reload keeps last known good effective configuration and emits an
  operator-visible error
- Missing `WORKFLOW.md` returns typed error
- Invalid YAML front matter returns typed error
- Front matter non-map returns typed error
- Config defaults apply when OPTIONAL values are missing
- `tracker.kind` validation enforces a known tracker adapter
- `cli.command` validation enforces a known CLI adapter
- `$VAR` resolution works for tracker auth and path values
- `~` path expansion works
- `cli.command` is preserved as a shell command string
- Per-state concurrency override map normalizes state names and ignores invalid values
- Prompt template renders `issue` and `attempt`
- Prompt rendering fails on unknown variables (strict mode)

### 17.2 Workspace Manager and Safety

- Deterministic workspace path per issue identifier
- Identifier sanitization replaces non-`[A-Za-z0-9._]` characters with `-`
- Missing workspace directory is created
- Existing workspace directory is reused
- Existing non-directory path at workspace location is handled safely (replace or fail per
  implementation policy)
- OPTIONAL workspace population/synchronization errors are surfaced
- `after_create` hook runs only on new workspace creation
- `before_run` hook runs before each attempt and failure/timeouts abort the current attempt
- `after_run` hook runs after each attempt and failure/timeouts are logged and ignored
- `before_remove` hook runs on cleanup and failures/timeouts are ignored
- Workspace path sanitization and root containment invariants are enforced before agent launch
- Agent launch uses the per-issue workspace path as cwd and rejects out-of-root paths
- Hook environment includes `SYMPHEO_ISSUE_IDENTIFIER`, `SYMPHEO_ISSUE_ID`, and
  `SYMPHEO_WORKSPACE_PATH`

### 17.3 Tracker Adapter Contract (Adapter Conformance)

These tests apply to every tracker adapter the implementation ships.

- `validate(tracker_config)` accepts a well-formed config and rejects missing auth / project
  identity
- `fetch_candidate_issues()` returns issues in active states only
- `fetch_issues_by_states([])` returns empty without an API call
- Pagination preserves order across multiple pages
- Blockers are normalized to a list of refs with `id`, `identifier`, and `state`
- Labels are normalized to lowercase
- `fetch_issue_states_by_ids` returns minimal normalized issues and silently omits missing IDs
- Error mapping covers transport failures, status errors, GraphQL errors, malformed payloads, and
  pagination integrity errors

### 17.4 GitHub Reference Adapter

- `tracker.kind = github` selects the GitHub adapter
- Validation requires `org`, `project_number`, `status_field`, and a non-empty resolved
  `auth_token`
- `auth_token` defaults to `$SYMPHEO_GITHUB_TOKEN` when not specified
- Identifier format is `<repo>#<number>`
- `branch_name` is generated from `<number>-<slugified-title>` and respects the 60-char truncation
- `state` is read from the configured `status_field` single-select option
- Issues without a value for `status_field` are ignored by dispatch and reconciliation
- Project membership filters out PRs and items without an associated issue
- Blockers are derived from GitHub Issue Dependencies (`trackedInIssues`)
- When Issue Dependencies are unavailable, the adapter falls back to body parsing
  (`Blocked by #N` / `Depends on #N`) and logs the fallback once
- HTTP requests use `Authorization: Bearer <token>` and the configured endpoint
- Pagination uses `pageInfo.endCursor` / `hasNextPage`

### 17.5 Orchestrator Dispatch, Reconciliation, and Retry

- Dispatch sort order is priority then oldest creation time
- An issue in the first active state with non-terminal blockers is not eligible
- An issue in the first active state with terminal blockers is eligible
- An issue in a later active state skips the blocker gate
- Active-state issue refresh updates running entry state
- Non-active state stops running agent without workspace cleanup
- Terminal state stops running agent and cleans workspace
- Reconciliation with no running issues is a no-op
- Normal worker exit schedules a short continuation retry (attempt 1)
- Abnormal worker exit increments retries with 10s-based exponential backoff
- Retry backoff cap uses configured `agent.max_retry_backoff_ms`
- Retry queue entries include attempt, due time, identifier, and error
- Stall detection kills stalled sessions and schedules retry
- Slot exhaustion requeues retries with explicit error reason
- If a snapshot API is implemented, it returns running rows, retry rows, token totals, and rate
  limits
- If a snapshot API is implemented, timeout/unavailable cases are surfaced

### 17.6 CLI Adapter Contract (Adapter Conformance)

These tests apply to every CLI adapter the implementation ships.

- Adapter selection picks the right adapter based on the leading binary token of `cli.command`
- `validate(cli_config)` checks binary discoverability and required `cli.options` keys
- `start_session` produces a session context with `session_id` and `agent_session_handle`
- `run_turn` launches the CLI subprocess in the per-issue workspace cwd
- `run_turn` rejects launches whose cwd would escape the workspace root
- `run_turn` enforces `cli.read_timeout_ms` and `cli.turn_timeout_ms`
- `run_turn` parses CLI output and emits normalized events
- The first turn uses the full prompt; subsequent turns send continuation guidance
- The same `agent_session_handle` is used across turns within one worker run
- Token usage events accumulate correctly (delta vs absolute is documented per adapter)
- Unsupported tool calls do not stall the session
- User-input-required signals are handled per the adapter's documented policy without indefinite
  stalling
- `stop_session` is safe to call after a `run_turn` failure

### 17.7 OpenCode Reference Adapter

- Default `cli.command` is `opencode run`
- The adapter detects the OpenCode binary by the leading token `opencode`
- Session resumption uses the OpenCode session-resume mechanism (for example `--session <handle>`)
- Output parsing extracts the final assistant message, tool calls, and token usage
- The adapter documents the OpenCode version range it has been tested against
- Unknown `cli.options` keys are ignored with a warning, not a failure

### 17.8 Observability

- Validation failures are operator-visible
- Structured logging includes issue/session context fields
- Logging sink failures do not crash orchestration
- Token/rate-limit aggregation remains correct across repeated agent updates
- If a human-readable status surface is implemented, it is driven from orchestrator state and does
  not affect correctness
- If humanized event summaries are implemented, they cover key event classes without changing
  orchestrator behavior

### 17.9 CLI and Host Lifecycle

- The Sympheo binary accepts a positional workflow path argument (`path-to-WORKFLOW.md`)
- The Sympheo binary uses `./WORKFLOW.md` when no workflow path argument is provided
- The Sympheo binary errors on nonexistent explicit workflow path or missing default `./WORKFLOW.md`
- The Sympheo binary surfaces startup failure cleanly
- The Sympheo binary exits with success when the application starts and shuts down normally
- The Sympheo binary exits nonzero when startup fails or the host process exits abnormally

### 17.10 Real Integration Profile (RECOMMENDED)

These checks are RECOMMENDED for production readiness and MAY be skipped in CI when credentials,
network access, or external service permissions are unavailable.

- A real GitHub tracker smoke test can be run with valid credentials supplied by
  `SYMPHEO_GITHUB_TOKEN` or a documented local bootstrap mechanism.
- A real OpenCode CLI smoke test runs `opencode run` end-to-end on a throwaway issue.
- Real integration tests SHOULD use isolated test identifiers/workspaces and clean up tracker
  artifacts when practical.
- A skipped real-integration test SHOULD be reported as skipped, not silently treated as passed.
- If a real-integration profile is explicitly enabled in CI or release validation, failures SHOULD
  fail that job.

## 18. Implementation Checklist (Definition of Done)

Use the same validation profiles as Section 17:

- Section 18.1 = `Core Conformance`
- Section 18.2 = `Adapter Conformance` for the reference adapters (GitHub + OpenCode)
- Section 18.3 = `Extension Conformance`
- Section 18.4 = `Real Integration Profile`

### 18.1 REQUIRED for Core Conformance

- Workflow path selection supports explicit runtime path and cwd default
- `WORKFLOW.md` loader with YAML front matter + prompt body split
- Typed config layer with defaults and `$` resolution
- Dynamic `WORKFLOW.md` watch/reload/re-apply for config and prompt
- Polling orchestrator with single-authority mutable state
- Tracker adapter contract honored: `validate`, `fetch_candidate_issues`,
  `fetch_issues_by_states`, `fetch_issue_states_by_ids`
- CLI adapter contract honored: `validate`, `start_session`, `run_turn`, `stop_session`
- Adapter resolution from `tracker.kind` and `cli.command`
- Workspace manager with sanitized per-issue workspaces (`.`, `_`, alphanumerics; other characters
  replaced with `-`)
- Workspace lifecycle hooks (`after_create`, `before_run`, `after_run`, `before_remove`)
- Hook timeout config (`hooks.timeout_ms`, default `60000`)
- CLI subprocess launch in workspace cwd with `bash -lc <cli.command> ...`
- Strict prompt rendering with `issue` and `attempt` variables
- Exponential retry queue with continuation retries after normal exit
- Configurable retry backoff cap (`agent.max_retry_backoff_ms`, default 5m)
- Reconciliation that stops runs on terminal/non-active tracker states
- Workspace cleanup for terminal issues (startup sweep + active transition)
- Structured logs with `issue_id`, `issue_identifier`, and `session_id`
- Operator-visible observability (structured logs; OPTIONAL snapshot/status surface)

### 18.2 REQUIRED for Reference Adapter Conformance

GitHub tracker adapter:

- Validates `org`, `project_number`, `status_field`, and resolved `auth_token`
- Default `auth_token` resolves from `$SYMPHEO_GITHUB_TOKEN`
- Identifier format `<repo>#<number>` and workspace key `<repo>-<number>`
- Branch name generated from `<number>-<slugified-title>`
- Status read from configured single-select field
- Project membership filters PRs and items without issues
- Blockers via GitHub Issue Dependencies with documented body-parsing fallback

OpenCode CLI adapter:

- Recognized when leading token of `cli.command` is `opencode`
- Default `cli.command` is `opencode run`
- Allocates and reuses `agent_session_handle` across turns within one worker run
- First turn uses full prompt; subsequent turns use continuation guidance
- Parses CLI output to emit normalized events (`session_started`, `turn_completed`,
  `turn_failed`, etc.)
- Documents OpenCode version range tested against

### 18.3 RECOMMENDED Extensions (Not REQUIRED for Core Conformance)

- HTTP server extension honors CLI `--port` over `server.port`, uses a safe default bind host, and
  exposes the baseline endpoints/error semantics in Section 13.7 if shipped.
- `github_graphql` client-side tool extension exposes raw GitHub GraphQL access through the
  agent session using configured Sympheo auth.
- Linear tracker adapter (Appendix B).
- Persist retry queue and session metadata across process restarts.
- First-class tracker write APIs (comments/state transitions) in the orchestrator instead of only
  via agent tools.
- Additional CLI adapters (for example `pi.dev`).
- SSH worker extension (Appendix A).

### 18.4 Operational Validation Before Production (RECOMMENDED)

- Run the `Real Integration Profile` from Section 17.10 with valid credentials and network access.
- Verify hook execution and workflow path resolution on the target host OS/shell environment.
- If the OPTIONAL HTTP server is shipped, verify the configured port behavior and loopback/default
  bind expectations on the target environment.

## Appendix A. SSH Worker Extension (OPTIONAL)

This appendix describes a common extension profile in which Sympheo keeps one central
orchestrator but executes worker runs on one or more remote hosts over SSH.

Extension config:

- `worker.ssh_hosts` (list of SSH host strings, OPTIONAL)
  - When omitted, work runs locally.
- `worker.max_concurrent_agents_per_host` (positive integer, OPTIONAL)
  - Shared per-host cap applied across configured SSH hosts.

### A.1 Execution Model

- The orchestrator remains the single source of truth for polling, claims, retries, and
  reconciliation.
- `worker.ssh_hosts` provides the candidate SSH destinations for remote execution.
- Each worker run is assigned to one host at a time, and that host becomes part of the run's
  effective execution identity along with the issue workspace.
- `workspace.root` is interpreted on the remote host, not on the orchestrator host.
- The CLI adapter's `run_turn` launches the CLI over SSH stdio instead of as a local subprocess, so
  the orchestrator still owns the session lifecycle even though commands execute remotely.
- Continuation turns inside one worker lifetime SHOULD stay on the same host and workspace.
- A remote host SHOULD satisfy the same basic contract as a local worker environment: reachable
  shell, writable workspace root, coding-agent CLI executable, and any required auth or repository
  prerequisites.

### A.2 Scheduling Notes

- SSH hosts MAY be treated as a pool for dispatch.
- Implementations MAY prefer the previously used host on retries when that host is still
  available.
- `worker.max_concurrent_agents_per_host` is an OPTIONAL shared per-host cap across configured SSH
  hosts.
- When all SSH hosts are at capacity, dispatch SHOULD wait rather than silently falling back to a
  different execution mode.
- Implementations MAY fail over to another host when the original host is unavailable before work
  has meaningfully started.
- Once a run has already produced side effects, a transparent rerun on another host SHOULD be
  treated as a new attempt, not as invisible failover.

### A.3 Problems to Consider

- Remote environment drift:
  - Each host needs the expected shell environment, coding-agent CLI executable, auth, and
    repository prerequisites.
- Workspace locality:
  - Workspaces are usually host-local, so moving an issue to a different host is typically a cold
    restart unless shared storage exists.
- Path and command safety:
  - Remote path resolution, shell quoting, and workspace-boundary checks matter more once execution
    crosses a machine boundary.
- Startup and failover semantics:
  - Implementations SHOULD distinguish host-connectivity/startup failures from in-workspace agent
    failures so the same ticket is not accidentally re-executed on multiple hosts.
- Host health and saturation:
  - A dead or overloaded host SHOULD reduce available capacity, not cause duplicate execution or an
    accidental fallback to local work.
- Cleanup and observability:
  - Operators need to know which host owns a run, where its workspace lives, and whether cleanup
    happened on the right machine.

## Appendix B. Linear Tracker Adapter Extension (OPTIONAL)

This appendix documents the Linear tracker adapter as an extension. It is not REQUIRED for Core
Conformance.

### B.1 `tracker` Front Matter Fields (Linear)

- `kind` (string)
  - REQUIRED. Value: `linear`.
- `endpoint` (string, OPTIONAL)
  - Default: `https://api.linear.app/graphql`.
- `auth_token` (string, OPTIONAL)
  - Default: `$LINEAR_API_KEY`.
  - MAY be a literal token or `$VAR_NAME`. If `$VAR_NAME` resolves to an empty string, treat as
    missing.
- `project_slug` (string)
  - REQUIRED. Linear project `slugId`.
- `active_states` (list of strings)
  - Default: `["Todo", "In Progress"]`.
- `terminal_states` (list of strings)
  - Default: `["Closed", "Cancelled", "Canceled", "Duplicate", "Done"]`.

### B.2 Identifier Format

`issue.identifier` is the Linear ticket key, for example `ABC-123`. Workspace key sanitization
preserves it unchanged because all characters are valid.

### B.3 Branch Name

`issue.branch_name` uses the Linear-provided branch metadata when available; otherwise it is
generated using the same algorithm as the GitHub adapter (`<number>-<slugified-title>` style),
where `<number>` is the numeric portion of the Linear identifier.

### B.4 Query Semantics

- Candidate issue query filters project using `project: { slugId: { eq: $projectSlug } }`.
- Issue-state refresh query uses GraphQL issue IDs with variable type `[ID!]`.
- Pagination is REQUIRED; default page size: `50`.
- Network timeout: `30000 ms`.

### B.5 Blockers

Blockers are derived from inverse relations of type `blocks` on the issue.

### B.6 Validation

- `tracker.kind == "linear"`.
- `tracker.auth_token` resolves to a non-empty string.
- `tracker.project_slug` is non-empty.

### B.7 OPTIONAL `linear_graphql` Tool Extension

The Linear adapter MAY expose a `linear_graphql` client-side tool with the same contract as the
`github_graphql` extension in Section 11.6, targeting the configured Linear endpoint and auth.