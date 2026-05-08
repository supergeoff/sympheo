---
tracker:
  kind: github
  api_key: $GITHUB_TOKEN
  project_slug: supergeoff/sympheo
  project_number: 2
  active_states:
    - todo
    - in progress
    - review
  terminal_states:
    - closed
    - done
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
  port: 8080
---

You are working on issue {{ issue.identifier }}: {{ issue.title }}.

Description: {{ issue.description }}

The repository has been cloned into this workspace. Please analyze the issue, implement the necessary changes, and ensure tests pass.

When you start working, move this issue to the "In Progress" column in the GitHub project.
When you are done and ready for review, move it to the "Review" column.
When the task is fully completed, move it to the "Done" column.

You can use `gh` or `curl` with the `GITHUB_TOKEN` environment variable to interact with the GitHub API.
The project number is 2 for repository supergeoff/sympheo.

**IMPORTANT:** Do NOT run `sympheo`, `cargo run`, or any orchestrator commands. Do NOT start long-running background processes or servers. Focus only on implementing the requested changes.

{% if attempt %}This is retry/continuation attempt {{ attempt }}.{% endif %}
