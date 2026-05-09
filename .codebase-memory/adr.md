## ADR-001 — SPEC.md is the normative contract

**Status:** Accepted
**Date:** 2026-05-09

`/home/supergeoff/projects/sympheo/SPEC.md` (Draft v1, language-agnostic, RFC 2119) is the contract of the service. Before any architectural change, refactor, or new feature: read the relevant section first. If a feature contradicts SPEC, flag it and propose an SPEC-compatible placement (hook, optional extension, etc.) instead of coding the contradiction.

Critical invariants to remember:
- §2.2 + §11.5 + §15.1: Sympheo does NOT validate agent-claimed outcomes. That guarantee is the workflow prompt's / hooks' / downstream-review's responsibility, never the scheduler's.
- §9.5 safety invariants are mandatory: workspace path inside root, cwd == workspace_path, key sanitized.
- §10.1: CLI adapter selected by leading binary token of `cli.command`. Mismatch → typed `cli_adapter_not_found`.
- §10.2: `validate / start_session / run_turn / stop_session` is the adapter contract. Lifecycle separation by turn, not by stage.
- §13.7: HTTP server is OPTIONAL extension. Routes baseline `/`, `/api/v1/state`, `/api/v1/<id>`, `POST /api/v1/refresh`. HTMX/SSE are implementation-defined on top.
- §15.5: harness hardening (HOME/XDG scrub) is OPTIONAL — implemented in `src/workspace/isolation.rs`.

## ADR-002 — Local backend per-worker isolation model

**Status:** Implemented (P4)
**Date:** 2026-05-09
**Source:** `src/workspace/isolation.rs`, `src/agent/backend/local.rs`

For every CLI turn, `LocalBackend::run_turn`:

1. Provisions `<workspace>/.sympheo-home/` with subdirs mapped to `HOME`, `XDG_CONFIG_HOME` (`/.config`), `XDG_DATA_HOME` (`/.local/share`), `XDG_CACHE_HOME` (`/.cache`), `XDG_STATE_HOME` (`/.local/state`), and `<HOME>/.local/bin` first in PATH. Idempotent.
2. Calls `Command::env_clear()` and re-populates with a layered env map:
   - Layer 1: host passthrough whitelist (`LANG`, `LANGUAGE`, `LC_*`, `TERM`, `TZ`, `USER`, `LOGNAME`).
   - Layer 2: Sympheo-managed (HOME / XDG_*).
   - Layer 3: default PATH (`<HOME>/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`).
   - Layer 4: `cli.env` operator overrides — always wins.
3. Spawns `bash -lc <cli.command>` in the workspace directory.

Prevents: cross-worker contamination, host config contamination (`~/.config/opencode`), credential-shaped env leakage (`ANTHROPIC_API_KEY`, `AWS_ACCESS_KEY_ID`, `GITHUB_TOKEN`), arbitrary host PATH discovery.

Does NOT prevent: filesystem absolute-path access (`cat /etc/passwd`), network access. Use Daytona backend or container for stronger guarantees.

## ADR-003 — Extensions over SPEC.md Draft v1

**Status:** In production
**Date:** 2026-05-09

Implementation-only fields not in SPEC §5.3 (allowed under §5.3 forward-compatibility):

- `daytona.*` (object): alternate execution backend. Fields: `enabled`, `api_key`, `api_url` (default `https://api.daytona.io`), `target` (default `us`), `image`, `timeout_sec` (default 3600), `env`, `mode` (default `oneshot`), `repo_url`. Source: `src/agent/backend/daytona.rs`.
- `skills.mapping` (map<state,path>): per-state SKILL.md prepended to prompt. Source: `src/skills/`.
- `workspace.git_reset_strategy` (`stash` default | `clean`): pre-turn workspace reset policy.
- `workspace.repo_url`: git clone source on first creation.
- `agent.max_turns_per_state` (map): per-state turn cap inside one worker.
- `agent.max_retry_attempts` (i64, default 5): per-issue retry cap (SPEC defines backoff timing only).
- `agent.continuation_prompt` (string): override the §10.2.2 continuation guidance.
- `tracker.fetch_blocked_by` (bool, default false): toggle GitHub linked-items query for blockers (currently disabled by default — see ADR-004 / issue #142).

CLI invocation extensions:
- OpenCode adapter appends `--dangerously-skip-permissions` to satisfy §10.4 high-trust posture. Will move into `cli.options.permissions` so operators can opt out.
- Prompt injected via temp file (`bash -lc 'PROMPT=$(cat ...); ...'`) to bypass ARG_MAX and shell escape.

Lifecycle extensions:
- `LocalBackend` spawns CLI in its own process group (`setpgid(0,0)`), uses `killpg(SIGKILL)` on cancel/timeout. Process registry tracks all live PIDs and signal handlers cleanup on shutdown — no zombie agents (P3).
- Dashboard adds `POST /api/v1/<id>/cancel` (kill switch via HTMX) and `DELETE /api/v1/retry/<id>` (drain retry queue) — beyond §13.7 baseline (P6).
- `start_server_with_listener(listener, state)` exposed alongside `start_server(port, state)` so tests can pre-bind the kernel port without race (see ADR-010).

Removed in P1:
- `probe_opencode` pre-flight subprocess (`__sympheo_probe__` invocation): the 10s timeout fired systematically because opencode accepted the prompt. `cli.turn_timeout_ms` and `cli.stall_timeout_ms` cover the same hang detection without false positives.

## ADR-004 — SPEC §11.4 GitHub adapter open gaps — issue #142

**Status:** Open — tracked as github.com/supergeoff/sympheo#142
**Date:** 2026-05-09
**Source:** `src/tracker/github.rs`, audit P0

The GitHub tracker adapter has known gaps vs SPEC §11.4 that need closing for full conformance. Full ticket text: https://github.com/supergeoff/sympheo/issues/142

1. **§11.4.1 `tracker.status_field` not honored.** Hardcoded to `"Status"` (`github.rs:20, 286, 437`). SPEC requires it to be REQUIRED+configurable.
2. **§11.4.5 per-repo filtering.** `github.rs:306-308` filters issues by `self.repo`; SPEC says "no per-repo filtering at the adapter level". Issues from any repo referenced by the project should be eligible.
3. **§11.4.6 GitHub Issue Dependencies not implemented.** `trackedInIssues` / `trackedIssues` GraphQL fields not requested. No body-parsing fallback (`Blocked by #N` / `Depends on #N`). No logged-once warning. `tracker.fetch_blocked_by` toggle exists but field is empty even when on.
4. **§11.4.1 `tracker.priority_field` not implemented.** `issue.priority` is always null (`github.rs:394`).
5. **§11.4.1 endpoint default mismatch.** Default returns `https://api.github.com`; code appends `/graphql` (`github.rs:108`). SPEC default is `https://api.github.com/graphql`. Functional but misleading.
6. **§11.6 `github_graphql` tool extension absent.** No mechanism to expose raw GitHub GraphQL to coding agents using Sympheo's configured tracker auth.

## ADR-005 — SPEC §10 + §13.7 + §17.9 secondary gaps — issues #144, #145, #146

**Status:** Open — tracked as #144, #145, #146
**Date:** 2026-05-09

Three issues consolidate the remaining gaps to close before SPEC §5-17 fully passes.

### Issue #144 — SPEC §10 CLI adapter (https://github.com/supergeoff/sympheo/issues/144)

1. **§10.2 lifecycle trait incomplete.** `CliAdapter` trait at `src/agent/cli/mod.rs:26` only exposes `validate`. `start_session` / `run_turn` / `stop_session` remain co-located in `LocalBackend` / `DaytonaBackend` / `MockBackend` instead of being lifted into adapters. P2 follow-up.
2. **§10.2.2 / §10.5 `read_timeout_ms` not distinct from `turn_timeout_ms`.** A single timeout is enforced (`local.rs:208`). SPEC distinguishes per-stdout-read stall from total wall-time.
3. **§10.3 `agent_pid` absent from event payload** (`parser.rs:124-195`). Traceability gap.
4. **§10.6 unknown `cli.options` keys silently accepted.** SPEC SHOULD log a warning to surface operator typos.
5. **§10.2.2 `TurnResult` shape.** Today returns `success: bool`; SPEC wants an `outcome` enum (`succeeded`/`failed`/`cancelled`/`timed_out`) + structured `usage` + typed `error`.

### Issue #145 — SPEC §13.7 HTTP API (https://github.com/supergeoff/sympheo/issues/145)

1. **Snapshot field naming `cli_totals` → `agent_totals`** (`server/mod.rs:474-479`). Field semantics already match.
2. **`POST /api/v1/refresh` returns 200, SPEC says 202 Accepted.**
3. **`retrying[]` rows missing `due_at` field** (`server/mod.rs:422-433`). Data exists internally (HTML dashboard shows it), surface in JSON.
4. **No JSON error envelope.** 404/etc return bare StatusCode; SPEC wants `{"error":{"code":"...","message":"..."}}`.
5. **No explicit 405 Method Not Allowed handler.** Axum returns 404 instead.
6. **`GET /api/v1/<issue_identifier>` shape** flattens fields SPEC nests (`attempts.{restart_count, current_retry_attempt}`, `running.tokens.{...}`). Realign without dropping extension fields.

### Issue #146 — SPEC §17.9 CLI binary lifecycle (https://github.com/supergeoff/sympheo/issues/146)

Six required binary behaviors (positional arg, default `./WORKFLOW.md`, error on missing path, clean startup-failure surfacing, exit 0 on normal shutdown, exit nonzero on failure) — none currently tested. Add `tests/cli_lifecycle.rs` using `assert_cmd` + mock CLI adapter + tmpdir workflow files (hermetic, zero-token).

Bundled smaller §17 holes to close in the same PR:
- §17.1 — strict-mode prompt rendering rejecting unknown variables.
- §17.1 — invalid WORKFLOW.md reload keeping the last known good config.
- §17.4 — `branch_name` 60-char truncation edge case.
- §17.5 — first-active-state-with-terminal-blockers IS eligible.

Closing #142, #144, #145, #146 brings SPEC §5-17 conformance to green.

## ADR-006 — Incident 2026-05-09 lessons

**Status:** Mitigated by P1-P7
**Date:** 2026-05-09

20 € of tokens burned on a 20-minute opencode run that transitioned a ticket Todo → Spec → In Progress → Review without producing any artifact (no PR, no branch, no commit, body unchanged). The orchestrator does not (and per SPEC §11.5 will not) verify artifacts.

Defense-in-depth implemented across PRs P1-P7:
- P1 — typed errors, removed noisy `probe_opencode`, lifecycle hardening.
- P3 — process registry + signal handlers → no zombie agents.
- P4 — workspace isolation + opencode stderr classifier (rate_limit / auth / account → typed errors instead of silent success).
- P5 — mock CLI adapter + scriptable backend (zero-token integration tests).
- P6 — dashboard kill switch + full last_message + HTMX (operator can detect "going into the wall" early).
- P7 — SPEC §17 conformance test map + e2e mock + real-integration gate.

Sympheo still does not validate artifacts in the orchestrator core (per SPEC). Validation responsibility is in the prompt (skills) and operator tooling (dashboard kill switch). No retry-on-empty-result either: if opencode claims success but produces nothing, Sympheo does not detect the discrepancy. The dashboard makes the empty turn visible to the operator as the only guard.

## ADR-007 — Codebase map

**Status:** Reference
**Date:** 2026-05-09

Top-level Rust modules and their SPEC alignment:

- `src/main.rs` — bootstrap, CLI args, file watcher, startup terminal cleanup (SPEC §8.6, §16.1).
- `src/lib.rs` — public surface.
- `src/error.rs` — typed error variants (SPEC §5.5, §10.5, §11.3).
- `src/workflow/{loader, parser, mod}.rs` — `WORKFLOW.md` discovery + YAML+prompt split (SPEC §5.1-5.4).
- `src/config/{resolver, typed, mod}.rs` — typed getters, `$VAR` resolution, defaults, validation (SPEC §6).
- `src/orchestrator/{mod, state, tick, retry}.rs` — single-authority state machine, poll loop, reconciliation, retry/backoff (SPEC §7-8, §16).
- `src/workspace/{mod, manager, isolation}.rs` — per-issue workspace lifecycle, hooks, safety invariants, env scrubbing (SPEC §9, §15.5).
- `src/agent/mod.rs` — agent runner orchestration.
- `src/agent/cli/{mod, opencode, pi, mock}.rs` — `CliAdapter` trait + adapters (SPEC §10.1, §10.6).
- `src/agent/backend/{mod, local, daytona, mock}.rs` — execution backends (subprocess + Daytona sandbox + scriptable mock).
- `src/agent/{runner, parser, process_registry}.rs` — worker algorithm, event parsing, PID tracking.
- `src/tracker/{mod, model, github, github/mutations}.rs` — `IssueTracker` trait + GitHub adapter (SPEC §11).
- `src/skills/{mod, loader, mapper}.rs` — per-state SKILL.md loading (extension).
- `src/server/mod.rs` — HTTP dashboard + API + kill switch (SPEC §13.7 + extensions). Exposes both `start_server(port, state)` (used by `main.rs`) and `start_server_with_listener(listener, state)` (used by tests).
- `src/git/{mod, adapter, local}.rs` — git workspace operations (extension).

Top-level test files in `tests/`:
- `integration_test.rs` — config/workflow integration.
- `orchestrator_test.rs` — dispatch, reconciliation, retry.
- `github_tracker_test.rs` — GitHub adapter against mock HTTP.
- `server_test.rs` — HTTP API contract. Uses `bind_test_server` helper to avoid the `find_free_port` race (ADR-010).
- `skills_test.rs` — skill loading.
- `e2e_mock.rs` — full e2e via mock backend (zero tokens, P5).
- `daytona_backend_test.rs` — Daytona adapter contract.
- `real_integration.rs` — gated by `SYMPHEO_REAL_INTEGRATION=1` env (P7, §17.10).

## ADR-008 — Pre-commit chain + branch / commit / memory conventions

**Status:** Enforced
**Date:** 2026-05-09

Before every push, run locally: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo check && cargo test`. The repo activates this via `.githooks/` (`git config core.hooksPath .githooks`).

Conventional Commits style for PR titles (matches existing history): `fix:`, `feat:`, `chore:`, `refactor:`, `test:`, `docs:`. Branches named `phase/p<N>-<slug>` for big phases or `feat/<slug>`, `docs/<slug>`, `chore/<slug>`, `fix/<slug>` otherwise.

Local tooling:
- `mise.toml` does not exist anymore; project tools now in `mise.local.toml` (gitignored, contains operator secrets — never touch, never commit).
- `.codebase-memory/adr.md` IS tracked (committed in PR #147). Maintain it via `mcp__codebase-memory-mcp__manage_adr` (mode=update, project=`home-supergeoff-projects-sympheo`). Each ADR update produces a doc-grade commit. The full graph (~1500 nodes, ~4700 edges) is rebuilt server-side via `index_repository` and is NOT committed.

Permission-mode patterns observed (auto mode):
- Direct commit on `main` is blocked by the harness — even for doc-only changes. Use a feature branch + PR + squash-merge always.
- `gh pr merge --squash --delete-branch` is allowed once a PR exists.
- `git reset --hard` and `git rebase` on `main` are blocked unless the user explicitly authorizes them (the user must name the branch / op they're authorizing).
- `mcp__github__issue_write` for issue creation is denied as an "External System Write" until the user explicitly authorizes ("go pour les issues" or equivalent).
- `gh pr create --merge` directly through `Bash` is fine; the MCP `create_pull_request` is the more idiomatic path.

## ADR-009 — Conformance roadmap to "all green"

**Status:** Active
**Date:** 2026-05-09

Closing this set of issues brings SPEC §5-17 conformance to green:

- #142 — §11.4 GitHub adapter (status_field, repo filter, blockers, priority_field, endpoint, github_graphql)
- #144 — §10 CLI adapter (lifecycle trait split, read_timeout vs turn_timeout, agent_pid, unknown options warning, TurnResult enum)
- #145 — §13.7 HTTP API (agent_totals, 202, due_at, error envelope, 405, issue detail nesting)
- #146 — §17.9 binary lifecycle tests + small §17.1 / §17.4 / §17.5 holes

After all four are closed, the conformance audit per the multi-agent review on 2026-05-09 flips to "all green" for §5-17. Real Integration Profile §17.10 remains opt-in via `SYMPHEO_REAL_INTEGRATION=1`.

When picking up an issue: read the relevant SPEC section first, write the failing test before the fix (TDD per `skills/build/SKILL.md`), check the local pre-commit chain (ADR-008) before push, open one PR per issue, wait for CI green via `gh pr checks <num> --watch`, squash-merge with `--delete-branch`, then `git fetch && git reset --hard origin/main` to align local main.

## ADR-010 — `find_free_port` race in server tests (PR #147 lesson)

**Status:** Fixed in PR #147
**Date:** 2026-05-09
**Source:** `src/server/mod.rs`, `tests/server_test.rs`

**Bug:** the original test helper
```rust
fn find_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}
```
returned a port number after **dropping** the listener. `start_server(port, ...)` then re-bound the port asynchronously. Between the drop and the re-bind, a parallel test could grab the same ephemeral port from the kernel, causing intermittent `Address already in use (os error 98)`. The flake was non-deterministic (different test cassait à chaque run) which made it look like a real regression.

**Wrong fixes to avoid:**
- Adding `--no-verify` to bypass the pre-commit chain (forbidden per ADR-008).
- Sleeping or retrying the bind (papers over the race, doesn't fix it).
- Adding `serial_test` crate (extra dep for one test file).
- Hardcoding port ranges (collisions when the dev box has many things bound).

**Correct pattern**, applied in PR #147:
1. Tests pre-bind the listener inside their own process: `let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap()`. The kernel keeps the binding alive as long as the listener lives.
2. Hand the listener (not the port) to a new `start_server_with_listener(listener, state)` entrypoint. Same axum app, no re-bind.
3. `start_server(port, state)` is preserved and now delegates to `start_server_with_listener` after binding — `main.rs` is unchanged.
4. The test helper `bind_test_server(state)` returns the port the listener is bound to and spawns the server task in one shot, with a small `sleep(200ms)` to let axum start serving before the test fires its first `reqwest`.

Stable across 5 consecutive parallel runs after the fix. Apply the same "listener, not port" pattern to any future test that needs a local HTTP server.

## ADR-011 — Conversation log 2026-05-09 (session output summary)

**Status:** Reference
**Date:** 2026-05-09

What this session produced (so the next agent can pick up without re-doing it):

1. Multi-agent SPEC §5–§17 conformity audit. Results condensed into ADR-001..010.
2. PR #143 — doc cleanup (squash `565098c`). Removed `docs/internal/`, `docs/dashboard.md`, `docs/audit-spec-v1.md`, `docs/conformance-tests.md`, `docs/extensions.md`, `docs/isolation.md`, `docs/incident-2026-05-09.md`, `docs/superpowers/`. Refreshed every user guide. Renamed `04-configuration.md` section `codex` → `cli`. Added Trust Boundary section in 01. Absorbed dashboard.md content into 08. `mise.toml` deleted in favour of `mise.local.toml` (gitignored).
3. Issues filed: #142 (§11.4), #144 (§10), #145 (§13.7), #146 (§17.9).
4. PR #147 — codebase-memory artefact + flake fix (squash `1c3f63d`). Two commits: `fix(test): eliminate server_test parallel port race` + `docs(memory): commit codebase-memory ADR artefact`. `.codebase-memory/adr.md` now tracked.
5. Local cleanup: 5 stale branches force-deleted (`chore/ci-dedupe-pr-checks`, `fix/agents-config-override-and-skills-tightening`, `fix/dashboard-html-content-type-and-graphql`, `fix/parser-token-clamp`, `list`). 10 stale remote-tracking refs pruned. `git reset --hard origin/main` after explicit user authorization to align local main with `1c3f63d`.
6. Security flag (still open at end of session): the operator's `mise.local.toml` contains a literal `SYMPHEO_GITHUB_TOKEN=ghp_BbNEUhge...`. The token leaked into the conversation transcript. The operator was advised to revoke and reissue. `.gitignore` covers the file, so it never reached git.

Numerical status of the codebase at session end:
- Tests: 384 passing, 2 ignored (real-integration, env-gated).
- Pre-commit chain: green.
- Local main HEAD: `1c3f63d` (== origin/main).
- Open issues tracking conformance: #142, #144, #145, #146.
- MCP knowledge graph: ~1542 nodes, ~4762 edges.
