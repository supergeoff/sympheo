# Configuration Reference

Sympheo is configured entirely through a single `WORKFLOW.md` file. The file has two parts: a YAML front matter block (between `---` delimiters) and a Liquid prompt template that follows.

## File Format

```markdown
---
# YAML configuration
---

# Liquid prompt template
```

## YAML Configuration

### `tracker`

Configures the issue tracker. Currently only GitHub is supported.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `kind` | string | Yes | Tracker type. Must be `github`. |
| `api_key` | string | Yes | GitHub token. Supports env var interpolation: `$GITHUB_TOKEN`. |
| `project_slug` | string | Yes | Repository slug, e.g. `owner/repo`. |
| `project_number` | integer | Yes | GitHub Project board number. |
| `active_states` | list of strings | Yes | Board columns Sympheo should poll. Case-insensitive. |
| `terminal_states` | list of strings | Yes | Board columns that mean "done". Triggers cleanup. |

Example:

```yaml
tracker:
  kind: github
  api_key: $GITHUB_TOKEN
  project_slug: acme/widgets
  project_number: 3
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
```

### `polling`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `interval_ms` | integer | `30000` | Milliseconds between polls. Minimum sensible value is ~5000. |

### `workspace`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `root` | string | required | Base directory for local workspaces. Supports `~` expansion. |

### `hooks`

Shell scripts run at workspace lifecycle events.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `after_create` | string (multiline) | none | Runs after the workspace directory is created. Use this to clone the repo. |
| `before_run` | string (multiline) | none | Runs before the agent is launched. |
| `after_run` | string (multiline) | none | Runs after the agent exits. |
| `before_remove` | string (multiline) | none | Runs before the workspace is deleted. |
| `timeout_ms` | integer | `60000` | Maximum time (ms) each hook is allowed to run. |

Environment variables available in hooks:
- `GITHUB_TOKEN` (and any other variables you set before running Sympheo)
- Any variable referenced in the YAML via `$NAME` syntax

### `agent`

Controls orchestrator-wide agent behavior.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `max_concurrent_agents` | integer | `5` | Maximum agents running at the same time. |
| `max_turns` | integer | `10` | Maximum turns per issue before giving up. |
| `max_retry_backoff_ms` | integer | `300000` | Cap on retry backoff in milliseconds. |

### `codex`

Configures the agent binary and its timeouts.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `command` | string | `opencode run` | The command used to launch the agent. |
| `turn_timeout_ms` | integer | `3600000` | Hard limit for a single turn (1 hour). |
| `read_timeout_ms` | integer | `5000` | Timeout for reading output from the agent process. |
| `stall_timeout_ms` | integer | `300000` | Time without output before the agent is considered stalled (5 minutes). |

### `daytona` (optional)

Enables Daytona sandbox execution.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `enabled` | boolean | Yes | Set to `true` to use Daytona. |
| `api_key` | string | Yes | Daytona API key. Supports env var interpolation. |
| `server_url` | string | Yes | Daytona server URL. |
| `target` | string | Yes | Daytona target name. |

When `daytona.enabled` is `true`, Sympheo ignores the local `workspace.root` and creates Daytona sandboxes instead.

### `server`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `port` | integer | none | Port for the HTTP dashboard and API. If omitted, no server starts unless passed via `--port` CLI flag. |

### `skills`

Maps workflow states to skill files.

| Field | Type | Description |
|-------|------|-------------|
| `mapping` | map<string, string> | Keys are state names (lowercase), values are relative paths to `SKILL.md` files. |

Example:

```yaml
skills:
  mapping:
    spec: ./skills/spec/SKILL.md
    in progress: ./skills/build/SKILL.md
    review: ./skills/review/SKILL.md
    test: ./skills/test/SKILL.md
    doc: ./skills/doc/SKILL.md
```

States without a mapped skill still receive the base prompt template.

## Liquid Prompt Template

Everything after the closing `---` is interpreted as a [Liquid](https://shopify.github.io/liquid/) template. It is rendered once per turn and passed to the agent as the system prompt.

### Available Variables

| Variable | Type | Description |
|----------|------|-------------|
| `issue.identifier` | string | Human-readable issue ID (e.g., `#42`). |
| `issue.id` | string | Internal tracker ID. |
| `issue.title` | string | Issue title. |
| `issue.description` | string | Issue body text. |
| `issue.state` | string | Current board column name. |
| `issue.labels` | list | Label names attached to the issue. |
| `issue.priority` | integer | Priority value if set. |
| `issue.branch_name` | string | Associated branch name if any. |
| `issue.url` | string | Link to the issue. |
| `issue.blocked_by` | list | Blocker references. |
| `attempt` | integer | Retry attempt number (starts at 1). Only present on retries. |

### Example Template

```liquid
You are working on issue {{ issue.identifier }}: {{ issue.title }}.

Description: {{ issue.description }}

Current stage: {{ issue.state }}
Labels: {{ issue.labels | join: ", " }}

{% if attempt %}This is retry attempt {{ attempt }}. Please be extra careful.{% endif %}
```

## Environment Variable Interpolation

Any value in the YAML that starts with `$` is resolved from the environment at startup:

```yaml
api_key: $GITHUB_TOKEN
```

If the variable is unset, startup fails with a validation error.

## CLI Overrides

| Flag | Description |
|------|-------------|
| `sympheo <path>` | Path to `WORKFLOW.md`. Defaults to `./WORKFLOW.md`. |
| `--port <number>` | Overrides the `server.port` config value. |
