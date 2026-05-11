---
# ════════════════════════════════════════════════════════════════════════
#  WORKFLOW.md — Sympheo daemon configuration (annotated example)
# ════════════════════════════════════════════════════════════════════════
#
#  Sympheo is a daemon that picks up GitHub issues whose Project v2
#  status matches an `active_state`, dispatches a coding-agent CLI per
#  issue (`opencode`, `claude`, `mock-cli`, …), and releases the worker
#  when the issue reaches a `terminal_state`.
#
#  This file has two parts separated by `---`:
#    1. YAML front-matter (this section)  → daemon configuration.
#    2. Markdown body (after the closing `---`) → the *default* prompt
#       template used when no per-phase `prompt:` overrides it.
#
#  Hot reload: the file is re-read on every orchestrator tick. Save →
#  wait one `polling.interval_ms` → new behaviour is in effect. No
#  daemon restart needed.
#
#  Boot validation rejects the daemon if any of these are missing:
#    • tracker.kind == "github"
#    • tracker.api_key
#    • tracker.project_slug
#    • tracker.project_number
#    • cli.command resolves to a known adapter
#    • Every phases[].state ∈ active_states, no duplicates

# ── tracker ─────────────────────────────────────────────────────────────
# Where issues come from. Sympheo polls this source on every tick, fetches
# candidate issues, and reconciles their state against the running workers.
tracker:
  kind: github                                # required ; "github" is the only adapter wired up today
  endpoint: https://api.github.com            # optional ; default depends on kind
  api_key: $GITHUB_TOKEN                      # required ; `$VAR` is expanded at runtime
  project_slug: supergeoff/sympheo            # required ; <owner>/<repo>
  project_number: 2                           # required for github ; the Project v2 number
  fetch_blocked_by: true                      # optional ; if true, `Todo` issues blocked by non-terminal blockers are skipped

  # active_states & terminal_states MUST mirror the columns of the Project
  # v2 board. The supergeoff/sympheo board has eight columns today:
  #     Canceled · Todo · Spec · In Progress · Review · Test · Doc · Done
  # Sympheo dispatches workers only for issues whose state is in
  # active_states. terminal_states triggers worker release & workspace
  # cleanup. Comparison is case-insensitive everywhere — `In Progress`
  # matches `in progress`, `IN PROGRESS`, etc.
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
    - Closed
    - Duplicate

# ── polling ─────────────────────────────────────────────────────────────
# Orchestrator tick frequency. Lower = more reactive, but more GH API
# calls (each tick fetches candidates from the Project v2 board).
polling:
  interval_ms: 30000                          # default 30s ; floor 1000ms

# ── workspace ───────────────────────────────────────────────────────────
# Each worker gets its own checkout under <root>/<issue-identifier>/.
workspace:
  root: ./.sympheo/workspaces                 # path is resolved relative to the directory of WORKFLOW.md
  repo_url: https://github.com/supergeoff/sympheo.git
  git_reset_strategy: stash                   # how the workspace is cleaned between turns ; "stash" (default) | "hard" | "none"

# ── hooks ───────────────────────────────────────────────────────────────
# Shell scripts run at well-known lifecycle points. Hooks are looked up
# by name; the four names below are the only ones the daemon currently
# invokes:
#   • after_create   — when a fresh workspace dir is created
#   • before_run     — once, just before the worker's turn loop starts
#   • after_run      — once, after the worker's turn loop ends
#   • before_remove  — just before the workspace is destroyed
#
# Execution surface:
#   • shell  → `bash -lc <script>` (login shell)
#   • cwd    → the per-issue workspace directory
#   • env    → SYMPHEO_ISSUE_IDENTIFIER, SYMPHEO_ISSUE_ID, SYMPHEO_WORKSPACE_PATH
#              (+ SYMPHEO_PHASE_NAME for before_run / after_run only)
#   • timeout → hooks.timeout_ms (same cap for all)
hooks:
  timeout_ms: 60000                           # default 60s

  # ─── after_create ─────────────────────────────────────────────────────
  # Fires once, the first time the workspace dir is created. NOTE: only
  # runs as a FALLBACK when `workspace.repo_url` is unset — with a
  # repo_url the daemon clones via its built-in git adapter and SKIPS
  # this hook. Comment out `workspace.repo_url` if you want this hook
  # to do the clone.
  after_create: |
    set -euo pipefail
    # Clone the target repo into the (empty) workspace dir.
    git clone --depth 50 https://github.com/supergeoff/sympheo.git .
    # Activate the repo's own githooks (fmt+clippy pre-commit, conv-commits commit-msg, …)
    git config core.hooksPath .githooks
    # Warm caches when a fresh lockfile is present.
    if [ -f Cargo.lock ]; then cargo fetch --locked; fi
    if [ -f e2e/bun.lock ]; then (cd e2e && bun install --frozen-lockfile); fi

  # ─── before_run ───────────────────────────────────────────────────────
  # Fires ONCE per worker run, just before the turn loop starts. Use it
  # to make sure the working branch exists and is up to date for that
  # run.
  before_run: |
    set -euo pipefail
    branch="sympheo/${SYMPHEO_ISSUE_IDENTIFIER#\#}"
    git fetch origin --quiet
    if git show-ref --verify --quiet "refs/heads/${branch}"; then
      git checkout "${branch}"
      git pull --ff-only origin "${branch}" 2>/dev/null || true
    else
      git checkout -b "${branch}" origin/main
    fi

  # ─── after_run ────────────────────────────────────────────────────────
  # Fires ONCE per worker run, after the turn loop ends. Belt-and-braces:
  # `.githooks/pre-commit` already enforces fmt+clippy on every commit, so
  # this hook is only here in case the agent failed to commit and there is
  # still uncommitted output on disk. If the githooks did their job (which
  # they should), the if-block below is a no-op.
  after_run: |
    set -euo pipefail
    cargo fmt --all
    cargo clippy --all-targets --all-features -- -D warnings
    if ! git diff --quiet || ! git diff --cached --quiet; then
      git add -A
      git commit -m "wip(${SYMPHEO_PHASE_NAME}): turn output for ${SYMPHEO_ISSUE_IDENTIFIER}"
    fi

  # ─── before_remove ────────────────────────────────────────────────────
  # Fires once, just before the workspace dir is deleted. Push the final
  # state and open a PR so the work survives the workspace teardown.
  before_remove: |
    set -euo pipefail
    branch="sympheo/${SYMPHEO_ISSUE_IDENTIFIER#\#}"
    # Nothing to push if the workspace was created but never committed.
    if ! git rev-parse --verify HEAD >/dev/null 2>&1; then exit 0; fi
    if git log "origin/main..HEAD" --oneline 2>/dev/null | grep -q .; then
      git push -u origin "${branch}"
      if ! gh pr view "${branch}" >/dev/null 2>&1; then
        gh pr create --base main --head "${branch}" \
          --title "feat: resolve ${SYMPHEO_ISSUE_IDENTIFIER}" \
          --body "Closes ${SYMPHEO_ISSUE_IDENTIFIER}"
      fi
    fi
    # The daemon removes ${SYMPHEO_WORKSPACE_PATH} right after this hook
    # returns. Clean any per-issue cache that lives outside that tree.
    rm -rf "/tmp/sympheo-cache-${SYMPHEO_ISSUE_ID}" 2>/dev/null || true

# ── agent ───────────────────────────────────────────────────────────────
# Concurrency, retry, and turn budget knobs.
agent:
  max_concurrent_agents: 3                    # global cap (default 10, min 1)
  max_turns: 20                               # per-worker turn budget (default 20, min 1)

  # Per-state turn budget. Overrides `max_turns` for issues in those
  # states. Keep the spec/review/doc lanes short so an over-eager agent
  # cannot burn the whole budget on prose.
  max_turns_per_state:
    todo: 1
    spec: 4
    "in progress": 30
    review: 8
    test: 6
    doc: 4

  # Per-state concurrency cap. Lets you keep at most N workers in a
  # given state at any time, on top of the global cap. Useful to keep
  # the high-cost `In Progress` lane single-file.
  max_concurrent_agents_by_state:
    todo: 5
    spec: 2
    "in progress": 1
    review: 2
    test: 2
    doc: 2

  max_retry_attempts: 5                       # max retries on failed turn (default 5, min 1)
  max_retry_backoff_ms: 300000                # max backoff cap, ms (default 5 min, floor 1000ms)

  # Prompt used by the worker to continue across turns inside the same
  # phase. Override to bias the agent's continuation style — e.g. force
  # it to re-read verification output before doing anything new.
  continuation_prompt: |
    Continue working on the current task. Re-read the conversation
    history, then proceed with the next step.

    If a phase verification failed on the previous turn, FIRST address
    the underlying problem. Never skip a verification by editing the
    workflow.

# ── cli ─────────────────────────────────────────────────────────────────
# How Sympheo invokes the coding-agent CLI. The leading binary token of
# `command` selects the adapter at boot. Four adapters are wired up
# today:
#
#   binary    | known cli.options keys
#   ----------+-----------------------------------
#   opencode  | model · permissions · mcp_servers
#   claude    | model · permission_mode · additional_args
#   mock-cli  | script
#   pi        | (stub — selection only, not runnable)
#
# Unknown option keys are forwarded verbatim and logged as a warning at
# session start, so adapters stay forward-compatible.
cli:
  command: claude                             # default is "opencode run"

  # Extra args appended to every turn invocation, AFTER the
  # adapter-specific flags. `$VAR` indirection works.
  args:
    - --dangerously-skip-permissions

  # Subprocess env. Merged on top of the daemon's process env. `$VAR`
  # indirection works. Use this for adapter API keys.
  env:
    ANTHROPIC_API_KEY: $ANTHROPIC_API_KEY

  # Adapter-specific options. Forwarded verbatim. Keys recognized by the
  # active adapter are listed in the table above; any other key is
  # forwarded too (with a warning log).
  options:
    model: claude-opus-4-7
    permission_mode: acceptEdits
    additional_args: ["--verbose"]

  turn_timeout_ms: 1800000                    # wall-clock per turn (default 3,600,000)
  read_timeout_ms: 5000                       # max wait between two stdout lines (default 5000)
  stall_timeout_ms: 300000                    # inactivity threshold before the worker is killed (default 300,000)

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
# Optional REST API for introspection & control. If omitted, no HTTP
# server is started — the daemon stays headless.
server:
  port: 9090

# ── phases ──────────────────────────────────────────────────────────────
# Each phase binds a Project v2 status to a per-state prompt,
# verifications, and per-phase `cli.options` overrides.
#
# Validation:
#   • `name`, `state`, `prompt` are required and non-empty.
#   • `phase.state` MUST be in tracker.active_states (case-insensitive).
#   • No two phases may share the same `state` (case-insensitive).
#   • An active_state without a matching phase falls back to the global
#     markdown body below — emits a warning at boot, not an error.
#
# Liquid template context — these are the ONLY root variables the strict
# validator accepts:
#   • `issue.*` — every field of the Issue struct exposed by the tracker:
#         id, identifier, title, description, priority, state,
#         branch_name, url, labels, blocked_by, node_id,
#         project_item_id, created_at, updated_at
#   • `attempt`  — retry counter (only set on retries; nil otherwise)
#   • `phase.name`, `phase.state`, `phase.prompt`
# Any other root variable raises a render error at runtime.
#
# Verifications: a list of shell commands run after the agent's turn
# finishes. A non-zero exit causes the worker to retry the turn (subject
# to `agent.max_retry_attempts`). Empty / whitespace entries are
# silently dropped. Each command runs in `bash -lc` with the same
# SYMPHEO_* env as the hooks above.
#
# Per-phase `cli_options` overlay the global `cli.options` for the
# duration of the phase. Use it to tighten permissions on spec-only
# phases and relax them on code phases.
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
  # 2) Spec — senior architect produces the design on the ticket itself
  # ──────────────────────────────────────────────────────────────────
  # NO production code is allowed here. The deliverable is a structured
  # "Architecture decision" comment posted DIRECTLY on the GitHub issue
  # — the ticket is the source of truth, not a side-channel doc tree.
  - name: spec
    state: Spec
    prompt: |
      You are the SENIOR ARCHITECT for issue {{ issue.identifier }}:
      {{ issue.title }}.

      Goal: produce the architectural blueprint a downstream `code` phase
      will implement. Do NOT write production code. Do NOT touch any
      source file. Your sole deliverable is a single "Architecture
      decision" comment posted on the GitHub issue itself.

      Use the GitHub CLI to post the comment:
        gh issue comment {{ issue.identifier }} --body-file - <<'MD'
        ... your architecture decision below ...
        MD

      The comment MUST contain these sections, with EXACTLY these
      headings so downstream verifications can grep them:

        ## Problem statement
        ## Scope / non-goals
        ## Options considered
        ## Decision & rationale
        ## Risks & open questions
        ## Acceptance criteria
        ## Test strategy

      Cover each section as a senior architect would:
        • Problem statement: what concrete user/operator pain are we
          solving, expressed in one paragraph.
        • Scope / non-goals: what is in, what is explicitly out — and
          why the non-goals are non-goals.
        • Options considered: 2–4 plausible designs, named, with a
          one-line summary each. No straw-men.
        • Decision & rationale: which option wins, and the decisive
          criteria. Mention the load-bearing trade-off.
        • Risks & open questions: anything that could turn the decision
          on its head; flag unknowns the implementer needs to resolve.
        • Acceptance criteria: a bullet list the implementer can tick
          off; testable, not aspirational.
        • Test strategy: which layer (unit / integration / e2e) covers
          which acceptance criterion.

      Do NOT move the issue forward — a human reviews the comment and
      flips the status to `In Progress` when satisfied.

      Issue body
      ----------
      {{ issue.description }}
    verifications:
      # The architecture comment must exist on the issue and contain
      # every required heading. Verifications run with cwd = workspace
      # and SYMPHEO_ISSUE_IDENTIFIER pre-populated.
      - |
        set -euo pipefail
        body=$(gh issue view "${SYMPHEO_ISSUE_IDENTIFIER}" \
                 --json comments --jq '.comments[].body')
        for heading in \
            '## Problem statement' \
            '## Scope / non-goals' \
            '## Options considered' \
            '## Decision & rationale' \
            '## Risks & open questions' \
            '## Acceptance criteria' \
            '## Test strategy' ; do
          printf '%s\n' "$body" | grep -qxF "$heading" \
            || { echo "missing heading on issue ${SYMPHEO_ISSUE_IDENTIFIER}: $heading" >&2 ; exit 1 ; }
        done
    cli_options:
      # Tighter permissions for spec authoring — the architect should
      # only call `gh`, never edit files.
      permission_mode: plan

  # ──────────────────────────────────────────────────────────────────
  # 3) In Progress — implement under RED-RED-GREEN TDD discipline
  # ──────────────────────────────────────────────────────────────────
  # Each acceptance criterion is delivered as a strict TDD cycle:
  #   • RED   — write the failing test FIRST.
  #   • RED   — run it, confirm it fails for the RIGHT reason.
  #   • GREEN — implement the smallest change that flips it green.
  # The second RED is the load-bearing step — it stops tests that pass
  # by accident from ever shipping. Skipping it is the single most
  # common way to ship a green test that verifies nothing.
  - name: code
    state: In Progress
    prompt: |
      You are entering the CODE phase of issue {{ issue.identifier }}:
      {{ issue.title }}.

      {% if attempt %}This is retry #{{ attempt }} — re-read the
      verification output from the previous turn before doing anything
      new.{% endif %}

      Read the "Architecture decision" comment posted during the spec
      phase — that is your blueprint:
        gh issue view {{ issue.identifier }} --json comments \
          --jq '.comments[].body'

      Work the Acceptance criteria checklist top-down. For EACH
      criterion, follow the RED-RED-GREEN discipline strictly:

        ┌─ RED ──────────────────────────────────────────────────────┐
        │ 1. Write the failing test FIRST.                           │
        │    • Use the test layer the architect prescribed in the   │
        │      "Test strategy" section (unit / integration / e2e).  │
        │    • The test encodes the criterion as an executable      │
        │      assertion — nothing more, nothing less.              │
        │    • Do NOT touch production code yet.                    │
        │    • Commit the test alone:                               │
        │        git commit -m "test(<scope>): cover <criterion>"   │
        └────────────────────────────────────────────────────────────┘

        ┌─ RED (again — diagnostic) ────────────────────────────────┐
        │ 2. Run the test and confirm it fails for the RIGHT reason.│
        │      cargo test --workspace --all-features <test_name>    │
        │    • The failure must come from the intended assertion —  │
        │      NOT a compile error, a typo, a missing import, or a  │
        │      panic in setup.                                      │
        │    • If the failure is for the wrong reason, fix the test │
        │      until the failure is meaningful, then re-run.        │
        │    • You stay in this step until the test fails the way   │
        │      you expect it to.                                    │
        └────────────────────────────────────────────────────────────┘

        ┌─ GREEN ────────────────────────────────────────────────────┐
        │ 3. Make the test pass with the SMALLEST possible change.  │
        │    • Add only the production code needed to flip this one │
        │      test from red to green.                              │
        │    • Re-run                                               │
        │        cargo test --workspace --all-features <test_name>  │
        │      then the full suite                                  │
        │        cargo test --workspace --all-features              │
        │      to prove nothing else broke.                         │
        │    • If a refactor is obvious now (rename / extract /     │
        │      dedupe), do it AFTER green and re-run the full       │
        │      suite again.                                         │
        │    • Commit:                                              │
        │        git commit -m "feat(<scope>): <criterion>"         │
        └────────────────────────────────────────────────────────────┘

      Loop until every acceptance criterion is green. Constraints:
        • Never have more than one RED test in the index at a time.
          If you discover a second criterion mid-flight, finish the
          current cycle first.
        • Never edit a test alongside the production code that makes
          it green — that breaks the discipline.
        • Never delete a failing test to "make CI green". Either fix
          the production code or push back on the architect comment.

      The before_run / after_run hooks above keep the branch alive
      across turns; before_remove opens the PR when the issue leaves
      the active states. You do NOT need to push or open the PR
      yourself.

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
      # TDD discipline guard: every commit on this branch since
      # origin/main that introduces a `feat:` or `fix:` MUST be
      # preceded by a `test:` commit on the same branch. Catches the
      # most common discipline lapse — implementation without a
      # corresponding red-first test.
      - |
        set -euo pipefail
        commits=$(git log --reverse --format='%s' origin/main..HEAD)
        seen_test=0
        echo "$commits" | while read -r subj; do
          case "$subj" in
            test:*|test\(*) seen_test=1 ;;
            feat:*|feat\(*|fix:*|fix\(*)
              [ "$seen_test" = 1 ] || {
                echo "TDD violation: '$subj' has no preceding test commit" >&2
                exit 1
              }
              seen_test=0
              ;;
          esac
        done

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
        • CHANGELOG.md — add to the Unreleased section
        • The architecture decision comment on the issue if the
          implementation deviated from the original design — append
          a "## Implementation notes" section there, do not rewrite
          the original decision.

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
  dispatches a worker with an empty prompt.
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
