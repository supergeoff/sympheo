---
tracker:
  kind: github
  api_key: $GITHUB_TOKEN
  project_slug: supergeoff/sympheo
  project_number: 2
  active_states:
    - todo
    - spec
    - in progress
    - review
    - test
    - doc
  terminal_states:
    - done
    - closed
    - cancelled

polling:
  interval_ms: 30000

workspace:
  root: ~/sympheo_workspaces

hooks:
  after_create: |
    git clone https://x-access-token:${GITHUB_TOKEN}@github.com/supergeoff/sympheo.git .
  before_run: |
    echo "Starting run"
  after_run: |
    echo "Run finished"
  before_remove: |
    echo "Removing workspace"
  timeout_ms: 60000

agent:
  max_concurrent_agents: 1
  max_turns: 10
  max_retry_backoff_ms: 300000

cli:
  command: opencode run
  env:
    # Forwarded from the env of the process that launched sympheo.
    # The worker runs under a scrubbed env (HOME / XDG_*_HOME redirected to
    # the per-issue workspace), so anything the agent CLI needs from the host
    # MUST be declared here. PATH itself is auto-built from mise's shims +
    # binary dirs — every tool you install via mise is automatically reachable.
    GITHUB_TOKEN: $GITHUB_TOKEN
    OPENROUTER_API_KEY: $OPENROUTER_API_KEY
  # Hard ceiling for a single agent turn. Set well above the longest
  # plausible spec/build session.
  turn_timeout_ms: 3600000
  # stdin/stderr probe timeout; kept tight because it's a startup check.
  read_timeout_ms: 5000
  # If no event has been observed for this many ms, the worker is
  # considered stalled and forcibly killed. Must be larger than the
  # silent-thinking gap of a model on a complex turn — 30 min covers
  # planning + tool-call sequences without false positives.
  stall_timeout_ms: 1800000

server:
  port: 9090

skills:
  mapping:
    todo: ./skills/todo/SKILL.md
    spec: ./skills/spec/SKILL.md
    in progress: ./skills/build/SKILL.md
    review: ./skills/review/SKILL.md
    test: ./skills/test/SKILL.md
    doc: ./skills/doc/SKILL.md
---

You are working on issue {{ issue.identifier }}: {{ issue.title }}.

Description: {{ issue.description }}

{% case issue.state %}
{% when "todo" %}
Your task is to verify, complete and advance this ticket to the Spec stage. Do NOT write code, run tests, or modify source files.
{% when "spec" %}
Your task is to produce a complete Low-Level Design (LLD) for this issue. Do NOT write implementation code yet.
{% when "in progress" %}
Your task is to implement the LLD with TDD discipline. Work in a dedicated branch from now on.
{% when "review" %}
Your task is to perform a thorough code review of the implementation. Check for bugs, consistency, and test coverage.
{% when "test" %}
Your task is to run all tests, validate the implementation, and ensure everything passes.
{% when "doc" %}
Your task is to write or update documentation. When complete, open a PR and move this issue to Done.
{% endcase %}

The project board has the following columns and workflow:
- **Todo**: Issues waiting to be picked up. Verify, complete and move to Spec.
- **Spec**: Technical specification and design phase. When ready to implement, move to "In Progress".
- **In Progress**: Active development and implementation phase. Work in a dedicated branch from now on. When done, move to "Review".
- **Review**: Code review phase. When review passes, move to "Test".
- **Test**: Testing and validation phase. When tests pass, move to "Doc".
- **Doc**: Documentation phase. When documentation is complete, open a PR and move to "Done".
- **Done**: Completed and verified. This is the terminal state.

When you finish the work for the current column, move this issue to the next appropriate column in the GitHub project using the GitHub API with the `GITHUB_TOKEN` environment variable.
The project number is 2 for repository supergeoff/sympheo.

**IMPORTANT:** Do NOT run `sympheo`, `cargo run`, or any orchestrator commands. Do NOT start long-running background processes or servers. Focus only on the task for the current column.

{% if attempt %}This is retry/continuation attempt {{ attempt }}.{% endif %}
