# Sympheo — SPEC §17 conformance test map

This document maps the SPEC.md §17 test matrix to the Rust test suite, with
exact `cargo test --test <crate>::<module>::<test>` selectors. It is the
companion to `docs/audit-spec-v1.md` (P0) and is updated as the test suite
evolves.

## Profiles

- **Core Conformance** — REQUIRED for all conforming implementations. Runs
  in the default `cargo test`.
- **Adapter Conformance** — REQUIRED for each tracker / CLI adapter shipped.
  Runs in the default `cargo test`.
- **Extension Conformance** — REQUIRED for each OPTIONAL extension shipped.
  Runs in the default `cargo test`.
- **Real Integration Profile** — RECOMMENDED for production readiness; MAY
  be skipped when credentials/network unavailable. Gated behind
  `#[ignore = "Reason: …"]` and the `SYMPHEO_REAL_INTEGRATION=1` env var.

## §17.1 Workflow / config parsing (Core)

| Sub-requirement | Test |
|---|---|
| Path precedence (explicit > cwd default) | `tests/integration_test::test_workflow_loader_with_front_matter` |
| File watch / reload / re-apply | covered indirectly by `src/workflow/loader.rs` tests |
| Missing WORKFLOW.md → typed error | `src/workflow/parser.rs::tests` |
| Front matter not a map → typed error | `src/workflow/parser.rs::tests` |
| Defaults applied for missing OPTIONAL values | `tests/integration_test::test_service_config_defaults` |
| `tracker.kind` validation | `src/config/typed.rs::tests::test_validate_unsupported_tracker_kind` |
| `cli.command` validation | `src/config/typed.rs::tests::test_validate_empty_cli_command` |
| `$VAR` resolution | `src/config/typed.rs::tests::test_tracker_api_key_env_resolution` + `tests/integration_test::test_config_var_resolution` |
| Per-state concurrency override | `src/config/typed.rs::tests::test_max_concurrent_agents_by_state*` |
| Prompt template renders `issue` and `attempt` | `src/orchestrator/tick.rs::tests` |
| Prompt fails on unknown variables | `src/orchestrator/tick.rs::tests` |

## §17.2 Workspace + safety invariants (Core)

| Sub-requirement | Test |
|---|---|
| Deterministic workspace path per identifier | `src/workspace/manager.rs::tests::test_workspace_path` |
| Sanitization `[A-Za-z0-9._-]` → `-` | `src/workspace/manager.rs::tests::test_sanitize_identifier_basic` (P1) |
| Missing dir created | `src/workspace/manager.rs::tests::test_create_or_reuse_new` |
| Existing dir reused | `src/workspace/manager.rs::tests::test_create_or_reuse_existing` |
| `after_create` runs on new only | `src/workspace/manager.rs::tests::test_create_or_reuse_with_after_create_hook` |
| `before_remove` runs on cleanup; failure logged-ignored | `src/workspace/manager.rs::tests::test_remove_workspace_with_before_remove_hook` |
| Path containment under root | `src/workspace/manager.rs::tests::test_validate_inside_root_*` |
| `SYMPHEO_*` env vars exposed to hooks | `src/workspace/manager.rs::tests::test_run_hook_env_vars_exposed` (P1) |
| Inv 1: `cwd == workspace_path` enforced before launch | `src/agent/backend/local.rs` (canonicalize + assertion, P1) |

## §17.3 Tracker Adapter Contract (Adapter)

| Sub-requirement | Test |
|---|---|
| `validate(tracker_config)` accepts well-formed | `src/tracker/github.rs::tests::test_validate_ok` (P1) |
| `validate` rejects negative project_number | `src/tracker/github.rs::tests::test_validate_negative_project_number` (P1) |
| `fetch_candidate_issues` | `tests/github_tracker_test::test_fetch_candidate_issues` |
| `fetch_issues_by_states([])` empty no API call | `tests/github_tracker_test::test_fetch_issues_by_states_empty` |
| `fetch_issue_states_by_ids` minimal + omit missing | `tests/github_tracker_test::test_fetch_issue_states_by_ids` |
| Pagination preserves order | covered by `fetch_project_items` cursor loop (manual review) |
| Labels normalized to lowercase | `src/tracker/github.rs::tests::test_normalize_item_ok` |
| Blockers normalized | `src/tracker/github.rs::tests::test_normalize_item_*` |
| Error mapping | `tests/github_tracker_test::test_graphql_error_response` |

## §17.4 GitHub Reference Adapter

| Sub-requirement | Test |
|---|---|
| Identifier `<repo>#<number>` | `src/tracker/github.rs::tests::test_normalize_item_ok` (P1) |
| `branch_name` `<number>-<slug>` truncate 60 | `src/tracker/github.rs::tests::test_build_branch_name_*` (P1, 5 tests) |
| State from `status_field` single-select | `src/tracker/github.rs::tests::test_extract_status_*` |
| Auth Bearer + Accept header | covered by client construction in `GithubTracker::new` (manual review) |
| Pagination cursor | `tests/github_tracker_test::test_fetch_candidate_issues` |

## §17.5 Orchestrator dispatch / reconciliation / retry (Core)

| Sub-requirement | Test |
|---|---|
| Non-blocking issue dispatched | `tests/orchestrator_test::test_orchestrator_tick_dispatches_eligible_issue` |
| First-state + non-terminal blockers ineligible | `tests/orchestrator_test::test_orchestrator_tick_skips_blocked_todo` |
| Later-state skips blocker gate | `tests/orchestrator_test::test_orchestrator_tick_dispatches_non_todo` |
| Worker exit → continuation retry attempt 1 | `tests/orchestrator_test::test_orchestrator_tick_worker_completes` |
| Failure-driven exponential backoff | `tests/orchestrator_test::*backoff*` |
| Stall detection terminates worker | `tests/orchestrator_test::test_orchestrator_tick_reconcile_stall_with_session` |
| End-to-end with mock CLI (workspace + dispatch + continuation) | `tests/e2e_mock::test_e2e_mock_dispatch_and_continuation` (P7) |
| Operator-issued cancel mid-turn | `tests/e2e_mock::test_e2e_mock_cancel_via_state` (P7) |

## §17.6 CLI Adapter Contract (Adapter)

| Sub-requirement | Test |
|---|---|
| Adapter selection by leading binary token | `src/agent/cli/mod.rs::tests::test_select_adapter_*` (P1, 6 tests) |
| `validate(cli_config)` rejects empty / wrong binary | `src/agent/cli/{opencode,pi,mock}.rs::tests::test_validate_*` (P1 + P5) |
| `run_turn` launches in workspace cwd | `src/agent/backend/local.rs::tests::test_local_backend_validate_outside_root` |
| `run_turn` enforces `turn_timeout_ms` | `src/agent/backend/local.rs::tests::test_local_backend_run_turn_timeout` |
| Parses output and emits normalized events | `src/agent/parser.rs::tests::*` + `src/agent/backend/local.rs::tests::test_local_backend_run_turn_success` |
| First turn full prompt; continuation guidance later | covered by `continuation_prompt` accessor + worker loop (manual review) |
| Token usage accumulates correctly | `src/orchestrator/state.rs::tests` |
| User-input-required handled | `src/agent/backend/local.rs::tests::test_classify_stderr_line_account_required` (P4) |
| Stderr error signals surface as typed errors | `src/agent/backend/local.rs::tests::test_classify_stderr_line_*` (P4, 5 tests) |
| Mock adapter conformance | `src/agent/backend/mock.rs::tests::*` (P5, 5 tests) |

## §17.7 OpenCode Reference Adapter (Adapter)

| Sub-requirement | Test |
|---|---|
| Default `cli.command = "opencode run"` | `src/config/typed.rs::tests::test_cli_command_default` |
| Adapter detected by leading token `opencode` | `src/agent/cli/opencode.rs::tests::test_validate_*` (P1) |
| Output parsing extracts final msg + tool calls + tokens | `src/agent/parser.rs::tests::*` |
| Documented version range | `src/agent/cli/opencode.rs::SUPPORTED_OPENCODE_VERSION_RANGE` (P1) |

## §17.8 Observability (Core)

| Sub-requirement | Test |
|---|---|
| Validation failures operator-visible | covered by `src/main.rs` startup error path |
| Structured logging includes `issue_id`, `issue_identifier`, `session_id` | spans in `src/agent/backend/local.rs::run_turn` (manual review) |
| Token / rate-limit aggregation correct across updates | `src/orchestrator/state.rs::tests` |
| Snapshot API returns running rows + retry rows + totals | `tests/server_test::test_server_api_state_with_data` |

## §17.9 CLI / host lifecycle (Core)

| Sub-requirement | Test |
|---|---|
| Binary accepts positional workflow path | `src/main.rs::tests::test_cli_with_workflow_path` |
| Default `./WORKFLOW.md` | `src/main.rs::tests::test_cli_default` |
| Errors on nonexistent path | `tests/integration_test::*` |
| `--port` CLI override | `src/main.rs::tests::test_cli_with_port` |
| Both CLI args | `src/main.rs::tests::test_cli_with_both` |
| No-zombie shutdown | `src/agent/process_registry.rs::tests::test_terminate_all_async_kills_real_subprocess` (P3) |

## §17.10 Real Integration Profile (RECOMMENDED, gated)

Gated tests live in `tests/real_integration.rs`. They require:

- `SYMPHEO_REAL_INTEGRATION=1` in the environment
- For GitHub: `SYMPHEO_GITHUB_TOKEN` set + a throwaway project
- For OpenCode: `opencode` binary on `PATH` + a configured account

Run them with:

```bash
SYMPHEO_REAL_INTEGRATION=1 cargo test --test real_integration -- --ignored
```

| Test | Status |
|---|---|
| `real_integration_smoke_opencode` | Stub — wire local fixture before invoking. |
| `real_integration_smoke_github_tracker` | Stub — wire throwaway project + token. |

Per spec, "skipped real-integration test SHOULD be reported as skipped, not
silently treated as passed". `#[ignore = "Reason: …"]` reports the test as
ignored in cargo's summary; the env-var guard inside each test refuses to
spend tokens even if `--ignored` is invoked without the explicit opt-in.

## Coverage gates (CI)

The `coverage` job in `.github/workflows/ci.yml` runs `cargo llvm-cov` and
fails the build if line coverage drops below **80%**. This is the
quantitative companion to the §17 mapping above.

## What the test matrix does NOT cover

- **§11.4.6 blockers via `trackedInIssues`** — disabled in code (commit
  `a6cf8f1`). When restored, add tests for both the GraphQL path and the
  body-parsing fallback.
- **§10.2 separated lifecycle (`start_session` / `run_turn` / `stop_session`
  as distinct trait methods)** — currently co-located in executor backends.
  When migrated, add per-method conformance tests.
- **§13.7 SSE per worker** — not implemented. The kill switch + full
  `last_message` are in P6, but a streaming event endpoint is future work.
- **§A SSH worker extension** — not implemented (Daytona is conceptually
  similar but uses HTTP API, not SSH).
- **§B Linear tracker adapter** — not implemented (referenced in config
  defaults only).
