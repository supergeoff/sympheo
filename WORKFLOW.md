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
  root: ~/symphony_workspaces

hooks:
  after_create: |
    echo "Workspace created"
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

Please analyze the issue, implement the necessary changes, and ensure tests pass.
{% if attempt %}This is retry/continuation attempt {{ attempt }}.{% endif %}
