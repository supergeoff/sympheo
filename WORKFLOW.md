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
    # Halt on any error / undefined var / failing pipe stage — a half-cloned
    # workspace must never reach the agent.
    set -euo pipefail
    # Shallow clone the target repo into the (empty) workspace dir created
    # by the daemon. Depth 50 keeps `git blame` / history reasonable without
    # paying for the full repo history.
    git clone --depth 50 https://github.com/supergeoff/sympheo.git .
    # Activate the repo's own githooks (fmt+clippy pre-commit, conv-commits
    # commit-msg, …) so every commit the agent makes from this workspace
    # passes through them.
    git config core.hooksPath .githooks
    # Warm the Rust crate cache when the lockfile is present, so the first
    # `cargo` call inside the turn does not pay the cold-fetch tax.
    if [ -f Cargo.lock ]; then cargo fetch --locked; fi
    # Same idea for the e2e harness's Bun dependencies.
    if [ -f e2e/bun.lock ]; then (cd e2e && bun install --frozen-lockfile); fi

  # ─── before_run ───────────────────────────────────────────────────────
  # Fires ONCE per worker run, just before the turn loop starts. Use it
  # to make sure the working branch exists and is up to date for that
  # run.
  before_run: |
    # Fail-fast guard (same rationale as after_create).
    set -euo pipefail
    # Per-issue branch name. The leading `#` on GitHub identifiers is
    # stripped so the branch stays git-safe (`#42` → `sympheo/42`).
    branch="sympheo/${SYMPHEO_ISSUE_IDENTIFIER#\#}"
    # Refresh origin refs quietly so the lookup below sees the latest state
    # without flooding the hook log.
    git fetch origin --quiet
    # If the branch already exists locally, switch to it and fast-forward
    # from origin. The `|| true` swallows the "no upstream yet" error on
    # never-pushed branches — there is nothing to fast-forward.
    if git show-ref --verify --quiet "refs/heads/${branch}"; then
      git checkout "${branch}"
      git pull --ff-only origin "${branch}" 2>/dev/null || true
    else
      # First touch for this issue: branch off the current origin/main.
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
    # Best-effort format + lint even if the agent forgot. The pre-commit
    # hook already enforces both on every commit; this is a safety net for
    # the uncommitted-diff case caught just below.
    cargo fmt --all
    cargo clippy --all-targets --all-features -- -D warnings
    # If anything is still uncommitted (worktree OR index), wrap it in a
    # WIP commit so `before_remove` can push it. The first `git diff` checks
    # the worktree, the second (`--cached`) checks the index.
    if ! git diff --quiet || ! git diff --cached --quiet; then
      git add -A
      # SYMPHEO_PHASE_NAME pins which phase produced the WIP — useful when
      # archaeology comes from the PR log later.
      git commit -m "wip(${SYMPHEO_PHASE_NAME}): turn output for ${SYMPHEO_ISSUE_IDENTIFIER}"
    fi

  # ─── before_remove ────────────────────────────────────────────────────
  # Fires once, just before the workspace dir is deleted. Push the final
  # state and open a PR so the work survives the workspace teardown.
  before_remove: |
    set -euo pipefail
    # Same branch convention as before_run.
    branch="sympheo/${SYMPHEO_ISSUE_IDENTIFIER#\#}"
    # Brand-new workspace that never committed anything — nothing to push,
    # exit clean before the git-log call below would fail.
    if ! git rev-parse --verify HEAD >/dev/null 2>&1; then exit 0; fi
    # Only push / open the PR when the branch has at least one commit ahead
    # of origin/main. `grep -q .` succeeds iff the log is non-empty.
    if git log "origin/main..HEAD" --oneline 2>/dev/null | grep -q .; then
      # `-u` sets the upstream so any later push needs no ref args.
      git push -u origin "${branch}"
      # Open the PR only when none exists yet for this head branch — the
      # hook stays idempotent across re-runs.
      if ! gh pr view "${branch}" >/dev/null 2>&1; then
        gh pr create --base main --head "${branch}" \
          --title "feat: resolve ${SYMPHEO_ISSUE_IDENTIFIER}" \
          --body "Closes ${SYMPHEO_ISSUE_IDENTIFIER}"
      fi
    fi
    # The daemon removes ${SYMPHEO_WORKSPACE_PATH} right after this hook
    # returns; clean any per-issue cache that lives outside that tree.
    # Best-effort: errors are silenced so the cleanup never fails the run.
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
# `command` selects the adapter at boot. Four adapters are wired up:
#
#   binary    | cli.options
#   ----------+----------------------------------------------------------
#   opencode  | model, additional_args (permission has no native flag)
#   claude    | model, permission, additional_args
#   pi        | model, additional_args (permission has no native flag)
#   mock-cli  | script (adapter-specific; not in the shared triplet)
#
# `cli.options` is a typed triplet shared by every production adapter:
#   • model            — string, mapped to the adapter's `--model` flag
#   • permission       — one of plan|acceptEdits|bypassPermissions|default
#                        (claude maps to `--permission-mode`; opencode and
#                        pi have no native equivalent and log a warn)
#   • additional_args  — string[], appended verbatim (shell-escaped per
#                        token, with `$VAR` resolution)
#
# Unknown keys are silently ignored by the typed view (mock reads its
# `script` extra this way). Renamed legacy keys hard-fail at parse:
#   • permission_mode  -> use permission
#   • permissions      -> use permission (singular)
#   • mcp_servers      -> declare via the agent's own config file
#   • cli.args         -> use cli.options.additional_args
cli:
  command: claude                             # default is "opencode run"

  # Subprocess env. Merged on top of the daemon's process env. `$VAR`
  # indirection works. Use this for adapter API keys.
  env:
    ANTHROPIC_API_KEY: $ANTHROPIC_API_KEY

  # Shared typed triplet — see the table above for per-adapter projection.
  options:
    model: claude-opus-4-7
    permission: acceptEdits
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
#       additional_args: ["--print"]
#     turn_timeout_ms: 1800000
#
# To switch to pi.dev:
#
#   cli:
#     command: pi
#     options:
#       model: sonnet:high
#       additional_args: ["--thinking", "high"]
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
# verifications, and an optional `cli.options` override.
#
# ── Available placeholders ──
#
# In `prompt:` (Liquid template; strict mode — unknown root names raise
# TemplateRenderError at runtime):
#   {{ issue.id }}               opaque tracker id
#   {{ issue.identifier }}       human identifier (e.g. "#42")
#   {{ issue.title }}            issue title
#   {{ issue.description }}      full issue body (string or nil)
#   {{ issue.state }}            current Project v2 status
#   {{ issue.priority }}         integer or nil
#   {{ issue.url }}              web URL
#   {{ issue.branch_name }}      branch name from the tracker, or nil
#   {{ issue.labels }}           array — render inline with `| join: ", "`
#   {{ issue.blocked_by }}       array of {id, identifier, state}
#   {{ issue.node_id }}          GitHub node id, or nil
#   {{ issue.project_item_id }}  Project v2 item id, or nil
#   {{ issue.created_at }}       RFC 3339 timestamp
#   {{ issue.updated_at }}       RFC 3339 timestamp
#   {{ attempt }}                retry counter — nil on the first try;
#                                gate retry-only copy behind `{% if attempt %}`
#   {{ phase.name }}             this entry's `name:`
#   {{ phase.state }}            this entry's `state:`
#   {{ phase.prompt }}           this entry's `prompt:` (self-reference)
#
# In `verifications:` (bash -lc; cwd = workspace; SYMPHEO_* env):
#   $SYMPHEO_ISSUE_IDENTIFIER
#   $SYMPHEO_ISSUE_ID
#   $SYMPHEO_WORKSPACE_PATH
#   $SYMPHEO_PHASE_NAME          (also exposed in before_run / after_run)
#
# ── Validation ──
#   • `name`, `state`, `prompt` required and non-empty
#   • `phase.state` ∈ tracker.active_states (case-insensitive)
#   • No two phases may share the same `state`
#   • An active_state without a matching phase falls back to the markdown
#     body after the closing `---` (warn at boot, not an error)
#
# ── Verifications ──
#   Shell commands run after each turn. Non-zero exit triggers a retry
#   (bounded by agent.max_retry_attempts). Empty / whitespace entries
#   are dropped silently.
#
# ── Per-phase cli.options ──
#   Shallow-merged over the global `cli.options` for the duration of the
#   phase: keys set here REPLACE the global value, absent keys keep it.
#   Use it to tighten permissions on spec-only phases and relax them on
#   code phases.
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
      You picked up {{ issue.identifier }} ("{{ issue.title }}") from the backlog.
      URL: {{ issue.url }} · Priority: {{ issue.priority }} · Labels: {{ issue.labels | join: ", " }}

      Body:
      {{ issue.description }}

      Decide:
        • Shovel-ready → move the Project v2 status to `Spec` with
          `gh project item-edit ...` and stop. Open no file.
        • Under-specified (no acceptance criteria, no clear ask, conflicting
          requirements) → post ONE comment listing exactly what is missing
          and stop without changing the status.
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
      You are the senior architect for {{ issue.identifier }} ("{{ issue.title }}").

      Deliverable: ONE "Architecture decision" comment posted on the GitHub
      issue. No production code, no source-file edits.

      Post it with:
        gh issue comment {{ issue.identifier }} --body-file - <<'MD'
        ... decision below ...
        MD

      The comment MUST contain these headings verbatim (a verification
      greps them — drift them and the turn fails):

        ## Problem statement     — the concrete user/operator pain, one paragraph.
        ## Scope / non-goals     — what is in, what is out, and why.
        ## Options considered    — 2–4 named designs, one-line each. No straw-men.
        ## Decision & rationale  — which option wins, and the load-bearing trade-off.
        ## Risks & open questions — anything that could turn the decision on its head.
        ## Acceptance criteria   — testable bullets the implementer ticks off.
        ## Test strategy         — which layer (unit / integration / e2e) covers what.

      Issue body:
      {{ issue.description }}

      Do not advance the status — a human moves it to `In Progress`.
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
    cli:
      options:
        # Tighter permissions for spec authoring — the architect should
        # only call `gh`, never edit files.
        permission: plan

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
      You are in the CODE phase of {{ issue.identifier }} ("{{ issue.title }}").
      {% if attempt %}Retry #{{ attempt }} — re-read the previous turn's verification output before doing anything new.{% endif %}

      Read the architect's blueprint:
        gh issue view {{ issue.identifier }} --json comments --jq '.comments[].body'

      Work the Acceptance criteria top-down. For EACH criterion, follow
      strict RED-RED-GREEN:

        1. RED — write the failing test FIRST, in the layer prescribed by
           "Test strategy". No production-code edit yet. Commit alone:
             git commit -m "test(<scope>): cover <criterion>"
        2. RED (diagnostic) — run the test and confirm it fails for the
           RIGHT reason (the intended assertion, not a compile error /
           typo / missing import / setup panic). Stay here until the
           failure is meaningful.
             cargo test --workspace --all-features <test_name>
        3. GREEN — smallest production change that flips the test, then
           the full suite to prove nothing else broke. Commit:
             cargo test --workspace --all-features
             git commit -m "feat(<scope>): <criterion>"

      Rules:
        • Never more than one RED test in the index at a time. Finish the
          current cycle before starting the next criterion.
        • Never edit a test alongside the production code that makes it
          green — that breaks the discipline.
        • Never delete a failing test to make CI green; fix the production
          code or push back on the architect comment.

      before_run / after_run / before_remove handle branching, WIP commits,
      and the PR. Do not push or open the PR yourself. A human moves the
      issue to `Review` once CI is green.
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
      You are in the REVIEW phase of {{ issue.identifier }} ("{{ issue.title }}").
      {% if attempt %}Retry #{{ attempt }} — start by re-reading the previous turn's failure output.{% endif %}

      The PR is open. Read CI and reviewer feedback:
        gh pr view
        gh pr checks
        gh api repos/supergeoff/sympheo/pulls/<n>/comments

      Address every actionable comment with a fixup commit (no amend, no
      force-push). Reply to each thread you resolve.

      Stop once every actionable comment is resolved or explicitly replied to.
    verifications:
      # All required CI checks must be green before the issue can move on.
      - "gh pr checks --required"

  # ──────────────────────────────────────────────────────────────────
  # 5) Test — post-merge end-to-end validation
  # ──────────────────────────────────────────────────────────────────
  - name: test
    state: Test
    prompt: |
      You are in the TEST phase of {{ issue.identifier }} ("{{ issue.title }}").

      The PR is merged into `main`. Run the e2e harness against the live
      environment:
        cd e2e && bun run e2e

      On failure: open a follow-up issue with the failure log attached,
      link it to {{ issue.identifier }}, and do NOT revert the merge.
    verifications:
      - "cd e2e && bun run e2e --include happy_path"
    cli:
      options:
        # Test phase is read-heavy — use the cheaper model.
        model: claude-sonnet-4-6

  # ──────────────────────────────────────────────────────────────────
  # 6) Doc — finalise user-facing documentation
  # ──────────────────────────────────────────────────────────────────
  - name: doc
    state: Doc
    prompt: |
      You are in the DOC phase of {{ issue.identifier }} ("{{ issue.title }}").

      Update user-facing docs to match the merged change:
        • README.md — if user-visible behaviour changed
        • CHANGELOG.md — append to the Unreleased section
        • The issue's architecture decision comment — ONLY if the
          implementation diverged from the original; append a
          "## Implementation notes" section, do not rewrite the decision.

      Write present-tense, snapshot-style prose. State what the system IS,
      never what it used to be ("Sympheo reads…", never "Sympheo now
      reads…" or "Sympheo no longer reads…").

      Commit and push. A human moves the issue to `Done` when the doc PR
      merges.
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
