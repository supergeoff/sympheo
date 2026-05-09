# Sympheo extensions over SPEC.md

This document lists fields, behaviors, and code paths present in this Sympheo
implementation that go beyond the strict text of `SPEC.md` (Draft v1).
The spec authorizes extensions provided they are documented (§5.3 forward
compatibility note: "Extensions SHOULD document their field schema, defaults,
validation rules, and whether changes apply dynamically or require restart.").

The audit `docs/audit-spec-v1.md` (P0) lists the conformance gaps and how each
extension was treated in P1.

## Workflow front-matter extensions

### `daytona:` (object) — alternate execution backend

Enables a remote Daytona-managed sandbox as the execution environment instead
of the local subprocess executor.

| Field | Type | Default | Notes |
|---|---|---|---|
| `enabled` | bool | `false` | Toggles the Daytona backend. |
| `api_key` | string \| `$VAR` | required when `enabled=true` | Daytona API key. |
| `api_url` | string | `https://api.daytona.io` | API endpoint. |
| `target` | string | `us` | Daytona target region. |
| `image` | string | unset | Optional Docker image override. |
| `timeout_sec` | i64 | `3600` | Sandbox lifetime. |
| `env` | map<string,string> | `{}` | Extra env vars injected into the sandbox. |
| `mode` | string | `oneshot` | Sandbox lifecycle mode. |
| `repo_url` | string | unset | Git repo cloned on sandbox creation. |

Spec relation: §3.2 leaves the Execution Layer implementation-defined.
Daytona is one such implementation choice. SPEC Appendix A describes a related
SSH-based pattern; Daytona is conceptually similar but uses an HTTP API rather
than SSH.

### `skills:` (object) — per-state prompt augmentation

Loads stage-specific markdown files and prepends their content to the rendered
prompt.

| Field | Type | Default | Notes |
|---|---|---|---|
| `mapping` | map<string,string> | `{}` | Maps tracker state name → relative path of a SKILL.md file. |

Spec relation: not part of §5.3. Allowed under §5.3 forward-compatibility.

### `workspace.git_reset_strategy` (string) — git workspace reset policy

| Value | Behavior |
|---|---|
| `stash` (default) | `git stash` dirty changes before each turn. |
| `clean` | `git clean -fdx && git reset --hard <branch>`. |

Spec relation: workspace population/reset is implementation-defined per §9.3.

### `workspace.repo_url` (string) — git clone source

If set, the workspace is populated by `git clone <repo_url>` on first creation.
Otherwise `hooks.after_create` is the population hook.

Spec relation: §9.3 OPTIONAL workspace population.

### `agent.max_turns_per_state` (map<string,i64>) — per-state turn cap

Caps the number of turns within a single tracker state, in addition to
`agent.max_turns` (which is a worker-lifetime cap per §5.3.5).

Spec relation: not part of §5.3.5 strict schema. Compatible with §5.3
forward-compatibility.

### `agent.max_retry_attempts` (i64) — per-issue retry cap

Caps the number of retry attempts per issue. Defaults to `5`. The spec defines
backoff timing (`max_retry_backoff_ms`) but not a max-attempts cap.

Spec relation: §5.3.5 forward-compatible extension.

### `agent.continuation_prompt` (string) — custom continuation guidance

Overrides the default short prompt used for turns 2..N within a worker
lifetime (§10.2.2 continuation semantics).

Spec relation: §10.2.2 + §10.6 mention "a short continuation message such as
`Continue working on the issue.`". This field allows operators to customize
the exact wording.

### `tracker.fetch_blocked_by` (bool) — toggle blockers fetching

When `false` (default), `Issue.blocked_by` is always empty even if the
underlying tracker exposes a blocker relation. When `true`, the GitHub
adapter would query `linkedItems` (currently disabled by default after
commit `a6cf8f1` removed the GraphQL field).

Spec relation: §11.4.6 mandates blockers via `trackedInIssues` with body
fallback. Restoring this is on the P1 follow-up list.

### `server:` (object, §13.7 OPTIONAL extension)

| Field | Type | Default | Notes |
|---|---|---|---|
| `port` | u16 | unset | Enables the HTTP server extension. CLI `--port` overrides. |

Spec relation: §13.7 explicitly OPTIONAL. Bind defaults to loopback (`127.0.0.1`).

## CLI invocation extensions

### `--dangerously-skip-permissions` flag passed to opencode

The reference adapter currently appends `--dangerously-skip-permissions` to
every opencode invocation to satisfy §10.4 (no interactive approvals while
running unattended). This is the high-trust posture mentioned in §10.6.
A future migration will move this into `cli.options.permissions` so
operators can opt out.

Spec relation: §10.4 implementation-defined approval policy. §10.6
mentions "configure `opencode run` with permissive permissions" as the
high-trust example.

### Prompt injection via temp file (`bash -lc 'PROMPT=$(cat ...); ...'`)

Avoids both shell-escape ambiguity and ARG_MAX limits when the prompt is
large.

Spec relation: §10.6 leaves stdin/argv parsing strategy adapter-defined.

## Lifecycle extensions

### Process group + watchdog kill (SIGKILL after SIGTERM)

`LocalBackend` spawns CLI subprocesses in their own process group via
`setpgid(0,0)` and uses `killpg` to terminate the entire tree on cancel /
timeout. SIGKILL is the immediate signal (no SIGTERM grace period); a
follow-up phase (P3) adds a graceful SIGTERM → SIGKILL sequence.

Spec relation: §14 leaves the failure-handling implementation open.

## Removed in P1

These behaviors were present in the code before P1 and are removed because
they are not in the spec and added noise / cost without benefit:

- `probe_opencode` pre-flight subprocess (`local.rs:33-90` in the prior
  version): launched a dummy `__sympheo_probe__` invocation and waited 10s
  on stderr to detect arg-rejection. The 10s timeout fired systematically
  (since opencode accepted the prompt instead of rejecting it), producing
  a `pre-flight probe timed out, proceeding anyway` warning on every turn.
  `cli.turn_timeout_ms` and `cli.stall_timeout_ms` cover the same hang
  detection without the false-positive noise.

## Partial in P1

These items were partially addressed; full coverage is deferred to a later
phase as recorded in the audit Annex C:

- §10.2 CLI Adapter lifecycle separation (`start_session` / `run_turn` /
  `stop_session` as distinct trait methods). P1 introduces the
  `crate::agent::cli::CliAdapter` trait with selection by leading binary
  token (§10.1) and `kind` / `binary_names` / `validate` surface, but the
  per-turn invocation logic remains co-located in `LocalBackend` /
  `DaytonaBackend`. A follow-up phase will move the lifecycle into
  `OpencodeAdapter` and `PiAdapter`.
- §11.4.6 Blockers via `trackedInIssues` (GraphQL) — currently disabled.
  Body-parsing fallback (`Blocked by #N`) also pending.
