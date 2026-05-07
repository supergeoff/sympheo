# Symphonie

Implementation of the Symphony service specification in Rust, adapted for **GitHub Issues** and **Opencode**.

## Adaptations from SPEC.md

- **Tracker**: `github` instead of `linear`
  - `tracker.project_slug` format: `owner/repo`
  - `tracker.api_key` canonical env: `GITHUB_TOKEN`
  - Active/terminal states are derived from issue labels combined with open/closed state
- **Agent**: `opencode` instead of `codex`
  - Uses `opencode run --format json` as the JSONL protocol
  - Session continuation via `--session <session_id>`
  - Tokens extracted from `step_finish` events

## Quick Start

1. Create a `WORKFLOW.md` (see example in repo root)
2. Set your GitHub token:
   ```bash
   export GITHUB_TOKEN=ghp_xxx
   ```
3. Run:
   ```bash
   cargo run -- path/to/WORKFLOW.md
   ```
4. Optional HTTP dashboard:
   ```bash
   cargo run -- path/to/WORKFLOW.md --port 8080
   ```

## WORKFLOW.md Format

```yaml
---
tracker:
  kind: github
  api_key: $GITHUB_TOKEN
  project_slug: owner/repo
  active_states:
    - todo
    - in progress
  terminal_states:
    - closed
    - done

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
  timeout_ms: 60000

agent:
  max_concurrent_agents: 5
  max_turns: 10
  max_retry_backoff_ms: 300000

codex:
  command: opencode run
  turn_timeout_ms: 3600000
  stall_timeout_ms: 300000

server:
  port: 8080
---

You are working on issue {{ issue.identifier }}: {{ issue.title }}.
```

## Architecture

- `workflow/` — WORKFLOW.md loader and parser
- `config/` — Typed configuration with `$VAR` resolution
- `tracker/` — GitHub REST API client
- `workspace/` — Directory lifecycle and hooks
- `agent/` — Opencode runner and JSONL event parser
- `orchestrator/` — Poll loop, dispatch, retries, reconciliation
- `server/` — Optional HTTP observability API (axum)

## Testing

```bash
cargo test
```
