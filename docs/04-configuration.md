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
| `project_slug` | string | Yes | Repository slug, e.g. `owner/repo`. The `owner` becomes the project's GitHub org. |
| `project_number` | integer | Yes | GitHub Project (v2) number. |
| `active_states` | list of strings | Yes | Board columns Sympheo should poll. Case-insensitive. |
| `terminal_states` | list of strings | Yes | Board columns that mean "done". Triggers cleanup. |
| `fetch_blocked_by` | bool | No (default `false`) | When `true`, populate `issue.blocked_by` from GitHub linked items. Off by default — see notes below. |

**Status field name** — the implementation currently reads the ProjectV2 single-select field named `Status` to determine each issue's state. Custom field names are not yet wired through configuration (planned alignment with SPEC §11.4.4). Make sure your project has a column named `Status`.

**Issue priority** — `issue.priority` is always `null` in this release. The optional `priority_field` configured by SPEC §11.4.1 is not yet implemented.

**Blockers** — by default `issue.blocked_by` is empty. Setting `tracker.fetch_blocked_by: true` enables the linked-items query, but the GraphQL `trackedInIssues` field and `Blocked by #N` body fallback (SPEC §11.4.6) are not yet implemented; tracking remains best-effort.

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
| `max_concurrent_agents` | integer | `10` | Maximum agents running at the same time. |
| `max_concurrent_agents_by_state` | map<string,int> | `{}` | Optional per-state concurrency caps. Keys are normalized to lowercase. |
| `max_turns` | integer | `20` | Maximum turns per issue worker session. |
| `max_turns_per_state` | map<string,int> | `{}` | Optional per-state turn caps inside one worker session. Extension. |
| `max_retry_attempts` | integer | `5` | Maximum retry attempts per issue before the claim is released. Extension. |
| `max_retry_backoff_ms` | integer | `300000` | Cap on retry backoff in milliseconds. |
| `continuation_prompt` | string | implementation default | Override the short prompt sent on continuation turns (SPEC §10.2.2). Extension. |

### `cli`

Configures the coding-agent CLI adapter. Sympheo selects the adapter from the leading binary token of `cli.command` (per SPEC §10.1): `opencode` ⇒ OpenCode adapter, `pi` ⇒ pi.dev adapter, `mock-cli` ⇒ test adapter.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `command` | string | `opencode run` | The command used to launch the agent. Run via `bash -lc` in the workspace cwd. |
| `args` | list of strings | `[]` | Extra arguments appended to `command` for each turn. |
| `env` | map<string,string> | `{}` | Environment variables added to the subprocess. Values support `$VAR`. Operator overrides always win over the isolated default env (see [10-advanced-topics](10-advanced-topics.md#trust-boundary)). |
| `options` | map | `{}` | Adapter-specific opaque options (e.g. `model`, `permissions`). Forwarded verbatim to the adapter. Unknown keys are ignored. |
| `turn_timeout_ms` | integer | `3600000` | Hard limit for a single turn (1 hour). |
| `read_timeout_ms` | integer | `5000` | Timeout for individual read operations on the agent stdout. |
| `stall_timeout_ms` | integer | `300000` | Time without output before the agent is considered stalled (5 minutes). |

### `daytona` (optional)

Enables Daytona sandbox execution as an alternative to local subprocess.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | boolean | `false` | Set to `true` to use Daytona for all workers. |
| `api_key` | string | required when enabled | Daytona API key. Supports `$VAR` interpolation. |
| `api_url` | string | `https://api.daytona.io` | Daytona control-plane URL. |
| `target` | string | `us` | Daytona target region. |
| `image` | string | none | Optional Docker image override for the sandbox. |
| `timeout_sec` | integer | `3600` | Sandbox lifetime in seconds. |
| `env` | map<string,string> | `{}` | Extra env vars injected into the sandbox. |
| `mode` | string | `oneshot` | Sandbox lifecycle mode. |
| `repo_url` | string | none | Git repo cloned on sandbox creation. |

When `daytona.enabled` is `true`, Sympheo ignores the local `workspace.root` semantics and creates Daytona sandboxes instead. See [`07-execution-modes.md`](07-execution-modes.md) for the trade-offs.

### `server` (optional extension)

The HTTP dashboard and JSON API are an OPTIONAL extension (SPEC §13.7). When neither `server.port` nor `--port` is set, no HTTP server starts.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `port` | integer | none | Port for the HTTP dashboard and API. CLI `--port` takes precedence. The server binds loopback (`127.0.0.1`) by default. |

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

Per SPEC §6.1, only values that explicitly contain `$VAR_NAME` are resolved from the environment. Sympheo does not globally override YAML values from the environment.

```yaml
api_key: $GITHUB_TOKEN
```

If the variable is unset (or resolves to empty), Sympheo treats the field as missing and fails dispatch validation with an operator-visible error. The service keeps running with the last known good configuration.

## Hot Reload

Sympheo watches `WORKFLOW.md` for changes. When it changes, the new config and prompt template are re-applied without restart and affect future polls, dispatch decisions, and agent launches. In-flight agent sessions are not restarted.

If the new file is invalid, Sympheo logs an operator-visible error and keeps running with the previous good configuration.

Tracker-kind or CLI-adapter swaps may require a restart; in-flight sessions started under the previous adapter are allowed to finish.

## CLI Overrides

| Flag | Description |
|------|-------------|
| `sympheo <path>` | Path to `WORKFLOW.md`. Defaults to `./WORKFLOW.md` in the current working directory. |
| `--port <number>` | Overrides `server.port`. Enables the HTTP dashboard even when `server.port` is unset. |

## Going Beyond the Spec

Several front-matter fields extend `SPEC.md` Draft v1 and are part of this implementation only: `daytona`, `skills.mapping`, `workspace.repo_url`, `workspace.git_reset_strategy`, `agent.max_turns_per_state`, `agent.max_retry_attempts`, `agent.continuation_prompt`, `tracker.fetch_blocked_by`. They are documented inline in the relevant tables above and fall under SPEC §5.3 forward-compatibility ("Extensions SHOULD document their field schema, defaults, validation rules, and whether changes apply dynamically or require restart").
