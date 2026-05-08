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
  max_concurrent_agents: 5
  max_turns: 10
  max_retry_backoff_ms: 300000

codex:
  command: opencode run
  turn_timeout_ms: 3600000
  read_timeout_ms: 5000
  stall_timeout_ms: 300000

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

The repository has been cloned into this workspace. Please analyze the issue, implement the necessary changes, and ensure tests pass.

The project board has the following columns and workflow:
- **Todo**: Issues waiting to be picked up. No specific skill is applied here.
- **Spec**: Technical specification and design phase. When ready to implement, move to "In Progress".
- **In Progress**: Active development and implementation phase. Work in a dedicated branch from now on. When done, move to "Review".
- **Review**: Code review phase. When review passes, move to "Test".
- **Test**: Testing and validation phase. When tests pass, move to "Doc".
- **Doc**: Documentation phase. When documentation is complete, open a PR and move to "Done".
- **Done**: Completed and verified. This is the terminal state.

When you finish the work for the current column, move this issue to the next appropriate column in the GitHub project using `gh` or the GitHub API with the `GITHUB_TOKEN` environment variable.
The project number is 2 for repository supergeoff/sympheo.

**IMPORTANT:** Do NOT run `sympheo`, `cargo run`, or any orchestrator commands. Do NOT start long-running background processes or servers. Focus only on implementing the requested changes.

{% if attempt %}This is retry/continuation attempt {{ attempt }}.{% endif %}
