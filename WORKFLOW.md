---
# ════════════════════════════════════════════════════════════════════════
#  WORKFLOW.md — Sympheo daemon configuration (annotated example)
# ════════════════════════════════════════════════════════════════════════
#
#  Sympheo is a Rust daemon that picks up GitHub issues whose Project v2
#  status matches an `active_state`, dispatches a coding-agent CLI per
#  issue (`opencode`, `claude`, `mock-cli`, …), and releases the worker
#  when the issue reaches a `terminal_state`.
#
#  This file has two parts separated by `---`:
#    1. YAML front-matter (this section)  → daemon configuration.
#    2. Markdown body (after the closing `---`) → the *default* prompt
#       template used when no per-phase `prompt:` overrides it.
#
#  Schema source of truth — read these files if anything below is unclear:
#    • Front-matter parser     src/workflow/parser.rs:4-30
#    • Typed accessors         src/config/typed.rs:30-303
#    • Phase block             src/workflow/phase.rs:9-101
#    • CLI adapter selection   src/agent/cli/mod.rs:244-271
#    • Prompt rendering        src/orchestrator/tick.rs:998-1063
#    • Issue struct (template) src/tracker/model.rs:11-27
#
#  Hot reload: the file is re-read on every orchestrator tick
#  (src/orchestrator/tick.rs:48-53). Save → wait one `polling.interval_ms`
#  → new behaviour is in effect. No daemon restart needed.
#
#  Boot validation rejects the daemon if any of these are missing:
#    • tracker.kind == "github"           (typed.rs:311)
#    • tracker.api_key                    (typed.rs:314)
#    • tracker.project_slug               (typed.rs:317)
#    • tracker.project_number             (typed.rs:320)
#    • cli.command resolves to an adapter (typed.rs:333)
#    • Every phases[].state ∈ active_states, no duplicates (phase.rs:66-92)

# ── tracker ─────────────────────────────────────────────────────────────
# Where issues come from. Sympheo polls this source on every tick, fetches
# candidate issues, and reconciles their state against the running workers.
# (src/config/typed.rs:54-117)
tracker:
  kind: github                                # required ; "github" is the only adapter wired up today (typed.rs:311)
  endpoint: https://api.github.com            # optional ; default per kind (typed.rs:58-67)
  api_key: $GITHUB_TOKEN                      # required ; `$VAR` is expanded at runtime (typed.rs:70-75)
  project_slug: supergeoff/sympheo            # required ; <owner>/<repo>
  project_number: 2                           # required for github ; Project v2 number (https://github.com/users/supergeoff/projects/2)
  fetch_blocked_by: true                      # optional ; if true, `Todo` issues blocked by non-terminal blockers are skipped (typed.rs:87-91)

  # active_states & terminal_states MUST mirror the columns of the Project
  # v2 board. The supergeoff/sympheo board has eight columns today:
  #     Canceled · Todo · Spec · In Progress · Review · Test · Doc · Done
  # Sympheo dispatches workers only for issues whose state is in
  # active_states. terminal_states triggers worker release & workspace
  # cleanup (typed.rs:102-117, tick.rs:78-107).
  #
  # Case-insensitive comparison everywhere — `In Progress` matches
  # `in progress`, `IN PROGRESS`, etc. (typed.rs:98, phase.rs:71-72).
  active_states:
    - Todo
    - Spec
    - In Progress
    - Review
    - Test
    - Doc
  terminal_states:
    - Done
    - Canceled
    - Cancelled                               # alias kept for cross-org consistency (typed.rs:108)
    - Closed
    - Duplicate

# ── polling ─────────────────────────────────────────────────────────────
# Orchestrator tick frequency. Lower = more reactive, but more GH API
# calls (each tick fetches candidates from the Project v2 board).
# (src/config/typed.rs:119-124)
polling:
  interval_ms: 30000                          # default 30s ; floor 1000ms

# ── workspace ───────────────────────────────────────────────────────────
# Each worker gets its own checkout under <root>/<issue-identifier>/.
# (src/workspace/manager.rs, src/config/typed.rs:126-154)
workspace:
  root: ./.sympheo/workspaces                 # path is resolved relative to the directory of WORKFLOW.md (typed.rs:126-138)
  repo_url: https://github.com/supergeoff/sympheo.git
  git_reset_strategy: stash                   # how the workspace is cleaned between turns ; "stash" (default) | "hard" | "none" (typed.rs:149-154)

# ── hooks ───────────────────────────────────────────────────────────────
# Shell scripts run at well-known lifecycle points. The same timeout
# applies to all of them (typed.rs:140-161). Hooks are looked up by name
# via `hook_script("<name>")` — only the names below are read today.
hooks:
  timeout_ms: 60000                           # default 60s
  after_create:   ./scripts/setup-workspace.sh
  before_run:     ./scripts/lint-before-run.sh
  after_run:      ./scripts/collect-artifacts.sh
  after_destroy:  ./scripts/cleanup-workspace.sh

# ── agent ───────────────────────────────────────────────────────────────
# Concurrency, retry, and turn budget knobs. (src/config/typed.rs:163-204, 214-230, 297-303)
agent:
  max_concurrent_agents: 3                    # global cap (default 10, min 1) (typed.rs:163-168)
  max_turns: 20                               # per-worker turn budget (default 20, min 1) (typed.rs:170-175)

  # Per-state turn budget. Overrides `max_turns` for issues in those
  # states (typed.rs:177-190). Keep the spec/review/doc lanes short so
  # an over-eager agent cannot burn the whole budget on prose.
  max_turns_per_state:
    todo: 1
    spec: 4
    "in progress": 30
    review: 8
    test: 6
    doc: 4

  # Per-state concurrency cap. Lets you keep at most N workers in a
  # given state at any time, on top of the global cap (typed.rs:214-230).
  # Useful to keep the high-cost `In Progress` lane single-file.
  max_concurrent_agents_by_state:
    todo: 5
    spec: 2
    "in progress": 1
    review: 2
    test: 2
    doc: 2

  max_retry_attempts: 5                       # max retries on failed turn (default 5, min 1) (typed.rs:199-203)
  max_retry_backoff_ms: 300000                # max backoff cap, ms (default 5 min, floor 1000ms) (typed.rs:192-196)

  # Prompt used by the worker to continue across turns inside the same
  # phase (typed.rs:297-303). Override to bias the agent's continuation
  # style — e.g. force it to re-read verification output before doing
  # anything new.
  continuation_prompt: |
    Continue working on the current task. Re-read the conversation
    history, then proceed with the next step.

    If a phase verification failed on the previous turn, FIRST address
    the underlying problem. Never skip a verification by editing the
    workflow.

# ── cli ─────────────────────────────────────────────────────────────────
# How Sympheo invokes the coding-agent CLI. The leading binary token of
# `command` selects the adapter at boot
# (src/agent/cli/mod.rs:244-271). Four adapters are wired up today:
#
#   binary    | adapter source             | known cli.options keys
#   ----------+----------------------------+-----------------------------------
#   opencode  | src/agent/cli/opencode.rs  | model · permissions · mcp_servers
#   claude    | src/agent/cli/claude.rs    | model · permission_mode · additional_args
#   mock-cli  | src/agent/cli/mock.rs      | script
#   pi        | src/agent/cli/pi.rs        | (stub — selection only, not runnable)
#
# Unknown option keys are forwarded verbatim and logged as a warning at
# `start_session` (cli/mod.rs:119-130), so adapters stay
# forward-compatible.
cli:
  command: claude                             # default is "opencode run" (typed.rs:232-236)

  # Extra args appended to every turn invocation, AFTER the
  # adapter-specific flags. `$VAR` indirection works (typed.rs:259-270).
  args:
    - --dangerously-skip-permissions

  # Subprocess env. Merged on top of the daemon's process env (typed.rs:274-286).
  # `$VAR` indirection works. Use this for adapter API keys.
  env:
    ANTHROPIC_API_KEY: $ANTHROPIC_API_KEY

  # Adapter-specific options. Forwarded verbatim (typed.rs:288-295).
  # Keys recognized by the active adapter are listed in the table above;
  # any other key is forwarded too (with a warning log).
  options:
    model: claude-opus-4-7
    permission_mode: acceptEdits
    additional_args: ["--verbose"]

  turn_timeout_ms: 1800000                    # wall-clock per turn (default 3,600,000) (typed.rs:238-243)
  read_timeout_ms: 5000                       # max wait between two stdout lines (default 5000) (typed.rs:245-250)
  stall_timeout_ms: 300000                    # inactivity threshold before the worker is killed (default 300,000) (typed.rs:252-257)

# ── Alternative adapter setups (commented out) ─────────────────────────
# To switch the daemon to OpenCode, replace the `cli:` block above with:
#
#   cli:
#     command: opencode run
#     options:
#       model: anthropic/claude-opus-4-7
#       permissions:
#         edit: true
#         bash: true
#       mcp_servers: []
#     turn_timeout_ms: 1800000
#
# For the mock pipeline (used by e2e tests — no real API spend):
#
#   cli:
#     command: mock-cli
#     options:
#       script: ./mock-events.yaml
#     turn_timeout_ms: 30000
#     stall_timeout_ms: 15000

# ── server ──────────────────────────────────────────────────────────────
# Optional REST API for introspection & control
# (src/server/, src/config/typed.rs:206-212). If omitted, no HTTP server
# is started — the daemon stays headless.
server:
  port: 8080

# ── phases ──────────────────────────────────────────────────────────────
# Each phase binds a Project v2 status to a per-state prompt,
# verifications, and per-phase `cli.options` overrides.
# (src/workflow/phase.rs:9-15, src/orchestrator/tick.rs:1045-1057)
#
# Validation (phase.rs:66-92):
#   • `name`, `state`, `prompt` are required and non-empty.
#   • `phase.state` MUST be in tracker.active_states (case-insensitive).
#   • No two phases may share the same `state` (case-insensitive).
#   • An active_state without a matching phase falls back to the global
#     markdown body below — emits a warning at boot, not an error.
#
# Liquid template context (src/orchestrator/tick.rs:1014-1057) — these
# are the ONLY root variables the strict validator accepts:
#   • `issue.*` — every field of `pub struct Issue`
#     (src/tracker/model.rs:11-27):
#         id, identifier, title, description, priority, state,
#         branch_name, url, labels, blocked_by, node_id,
#         project_item_id, created_at, updated_at
#   • `attempt`  — retry counter (only set on retries; nil otherwise)
#   • `phase.name`, `phase.state`, `phase.prompt`
# Any other root variable raises `TemplateRenderError` at runtime.
#
# Verifications (phase.rs:41-46): a list of shell commands run after
# the agent's turn finishes. A non-zero exit causes the worker to retry
# the turn (subject to `agent.max_retry_attempts`). Empty / whitespace
# entries are silently dropped.
#
# Per-phase `cli_options` (phase.rs:46) overlay the global `cli.options`
# for the duration of the phase. Use it to tighten permissions on
# spec-only phases and relax them on code phases.
phases:

  # ──────────────────────────────────────────────────────────────────
  # 1) Todo — handover gate
  # ──────────────────────────────────────────────────────────────────
  # The agent reads a freshly-prioritised ticket and decides whether
  # it is shovel-ready. If yes, it flips the status to `Spec` itself.
  # If under-specified, it leaves a comment and stops.
  - name: triage
    state: Todo
    prompt: |
      You just picked up issue {{ issue.identifier }} from the backlog.

      Title : {{ issue.title }}
      URL   : {{ issue.url }}
      Labels: {{ issue.labels | join: ", " }}

      Decide whether the ticket is ready for spec authoring.

        • If yes: flip its Project v2 status to `Spec` with
          `gh project item-edit ...` and stop. Do NOT open files.

        • If it is under-specified (no acceptance criteria, no clear
          ask, conflicting requirements): post a single comment listing
          exactly what is missing, then stop without changing status.

      Body
      ----
      {{ issue.description }}
    verifications:
      - "true"

  # ──────────────────────────────────────────────────────────────────
  # 2) Spec — translate the issue into an OpenSpec proposal
  # ──────────────────────────────────────────────────────────────────
  # NO production code is allowed here. Spec drift is the #1 cause of
  # runaway agents; the turn budget is tight on purpose.
  - name: spec
    state: Spec
    prompt: |
      You are entering the SPEC phase of issue {{ issue.identifier }}:
      {{ issue.title }}.

      Goal: produce (or update) an OpenSpec proposal at
      `openspec/changes/<change-name>/`. Do NOT write production code.
      Do NOT touch files outside `openspec/`.

      Deliverables this phase:
        • proposal.md   — problem, scope, non-goals, acceptance criteria
        • tasks.md      — ordered checklist
        • specs/<capability>/spec.md deltas if the proposal touches an
          existing capability

      When the proposal is ready, commit and push. Do NOT move the issue
      forward — a human reviews the proposal and flips the status to
      `In Progress`.

      Issue body
      ----------
      {{ issue.description }}
    verifications:
      # The two MUST files are the proposal and its tasks list.
      - "ls openspec/changes/*/proposal.md"
      - "ls openspec/changes/*/tasks.md"
      # OpenSpec's own validator catches malformed deltas early.
      - "openspec validate --strict"
    cli_options:
      # Tighter permissions for spec authoring — agent can only edit
      # openspec/, nothing else.
      permission_mode: plan

  # ──────────────────────────────────────────────────────────────────
  # 3) In Progress — code the OpenSpec proposal
  # ──────────────────────────────────────────────────────────────────
  - name: code
    state: In Progress
    prompt: |
      You are entering the CODE phase of issue {{ issue.identifier }}:
      {{ issue.title }}.

      {% if attempt %}This is retry #{{ attempt }} — re-read the
      verification output from the previous turn before doing anything
      new.{% endif %}

      Goal: implement the OpenSpec proposal under
      `openspec/changes/<change-name>/`. Follow `tasks.md` in order.
      After every task:
        1. Run `cargo fmt --all`
        2. Run `cargo clippy --all-targets --all-features -- -D warnings`
        3. Run `cargo test --workspace --all-features`
        4. Commit with a Conventional Commit message
           (`feat:`, `fix:`, `docs:`, `refactor:`, `chore:`, `test:`)

      When the implementation is complete:
        • Push the branch (`feat/<short-slug>`).
        • Open a PR against `main` with a Conventional Commit title and
          `closes #{{ issue.identifier }}` in the body.

      Do NOT move the issue forward — a human flips it to `Review`
      once CI is green.
    verifications:
      - "cargo fmt --all -- --check"
      - "cargo clippy --all-targets --all-features -- -D warnings"
      - "cargo test --workspace --all-features"
      # The CI also enforces these via .github/workflows/ci.yml — fail
      # fast locally so we don't burn a PR-checks round-trip.
      - ".githooks/lib/enforce-patterns.sh"
      - "scripts/check-coverage.sh lcov.info 80"

  # ──────────────────────────────────────────────────────────────────
  # 4) Review — address PR feedback
  # ──────────────────────────────────────────────────────────────────
  - name: review
    state: Review
    prompt: |
      You are entering the REVIEW phase of issue {{ issue.identifier }}.

      The PR is already open. Read CI logs and reviewer comments via:
        • `gh pr view`
        • `gh pr checks`
        • `gh api repos/supergeoff/sympheo/pulls/<n>/comments`

      Address every actionable comment with a fixup commit (do NOT
      amend; do NOT force-push). Reply to each thread you resolve.

      Stop when every actionable comment is either resolved or
      explicitly replied to.
    verifications:
      # All required CI checks must be green before the issue can move on.
      - "gh pr checks --required"

  # ──────────────────────────────────────────────────────────────────
  # 5) Test — post-merge end-to-end validation
  # ──────────────────────────────────────────────────────────────────
  - name: test
    state: Test
    prompt: |
      You are entering the TEST phase of issue {{ issue.identifier }}.

      The PR has merged into `main`. Run the e2e harness against the
      live environment:

        cd e2e && bun run e2e

      If anything fails: open a follow-up issue with the failure log
      attached, link it to {{ issue.identifier }}, and do NOT revert
      the merge.
    verifications:
      - "cd e2e && bun run e2e --include happy_path"
    cli_options:
      # Test phase is read-heavy — use the cheaper model.
      model: claude-sonnet-4-6

  # ──────────────────────────────────────────────────────────────────
  # 6) Doc — finalise user-facing documentation
  # ──────────────────────────────────────────────────────────────────
  - name: doc
    state: Doc
    prompt: |
      You are entering the DOC phase of issue {{ issue.identifier }}.

      Update user-facing docs to match the now-merged change:
        • README.md (if user-visible behaviour changed)
        • openspec/specs/<capability>/spec.md if the proposal had deltas
        • CHANGELOG.md — add to the Unreleased section

      Write present-tense, snapshot-style prose. Do NOT describe what
      used to be the behaviour, only what it is now ("Sympheo reads…",
      never "Sympheo now reads…" or "Sympheo no longer reads…").

      Commit and push. A human flips the issue to `Done` once the doc
      PR merges.
    verifications:
      - "test -f CHANGELOG.md"
      # Snapshot-style guard — block any "nouvelle", "retiré", "désormais",
      # "post-X" wording that betrays transitional prose.
      - "! grep -niE 'nouvelle|retir(é|e)|d[ée]sormais|post-[a-z0-9]+' README.md CHANGELOG.md"

---
{% comment %}
  Default prompt template (fallback).

  This body is rendered only for active_states that do NOT have a matching
  `phases[].state` entry above. With the `phases:` block declared above,
  every active state has its own prompt, so this body is effectively
  unused at runtime — it is kept as a safety net so the daemon never
  dispatches a worker with an empty prompt
  (src/orchestrator/tick.rs:1008-1012).

  Liquid template context (tick.rs:1014-1057) — see the long comment in
  the front-matter above for the full variable list. The strict
  validator (tick.rs:1015-1026) accepts only `issue`, `attempt`, `phase`
  as root variables.
{% endcomment %}

You are an autonomous coding agent picking up issue {{ issue.identifier }}.

Title : {{ issue.title }}
State : {{ issue.state }}
URL   : {{ issue.url }}
Phase : {{ phase.name }} ({{ phase.state }})
{% if attempt %}Attempt: #{{ attempt }}{% endif %}

The orchestrator did not find a phase prompt for this state. Read the
issue body, decide on a minimal next step, and stop after committing
your work. Do not move the issue's Project v2 status forward — a human
will inspect the workspace and decide what comes next.

Body
----
{{ issue.description }}
