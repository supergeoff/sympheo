# Advanced Topics

This section covers customization and operational patterns for experienced users.

## Custom Hooks for CI Integration

Hooks are not limited to simple echo statements. You can use them to integrate with external systems:

### Post-build notification

```yaml
hooks:
  after_run: |
    if [ -f test-results.xml ]; then
      curl -X POST \
        -H "Authorization: Bearer $CI_TOKEN" \
        -F "file=@test-results.xml" \
        https://ci.example.com/upload
    fi
```

### Slack notification on failure

```yaml
hooks:
  after_run: |
    if [ "$AGENT_EXIT_CODE" != "0" ]; then
      curl -X POST -H 'Content-type: application/json' \
        --data '{"text":"Agent failed for '"$ISSUE_IDENTIFIER"'"}' \
        $SLACK_WEBHOOK_URL
    fi
```

> Note: The `AGENT_EXIT_CODE` and `ISSUE_IDENTIFIER` variables are illustrative. Check the actual environment exposed by Sympheo; you may need to infer state from the workspace contents.

## Running Multiple Sympheo Instances

You can run multiple orchestrators against different projects or with different configurations:

```bash
# Instance A: local mode, small projects
cargo run -- WORKFLOW_SMALL.md --port 9090

# Instance B: Daytona mode, large projects
cargo run -- WORKFLOW_LARGE.md --port 9091
```

Each instance maintains its own state, dashboard, and workspace pool.

## Tuning Concurrency

The `agent.max_concurrent_agents` setting controls how many agents run in parallel. The right value depends on your hardware and the agent's resource usage:

| Scenario | Recommended `max_concurrent_agents` |
|----------|-----------------------------------|
| Local dev machine (8 cores) | 2–3 |
| CI runner (16 cores, 32 GB) | 5–8 |
| Daytona mode (unlimited target) | 10–20 |

If agents compete for the build cache or database locks, reduce concurrency. If CPU is idle, increase it.

## Custom Prompt Templates

The Liquid template in `WORKFLOW.md` is your primary lever for controlling agent behavior globally. Here are some advanced patterns:

### Conditional instructions per label

```liquid
{% for label in issue.labels %}
{% if label == "hotfix" %}
URGENT: This is a hotfix. Minimize changes and focus on correctness over elegance.
{% endif %}
{% endfor %}
```

### Branch-aware prompts

```liquid
{% if issue.branch_name %}
You are working on branch: {{ issue.branch_name }}.
Please base your changes on this branch and do not switch branches.
{% else %}
Create a new branch named `feature/{{ issue.identifier | replace: '#', '' }}`.
{% endif %}
```

### Attempt-aware escalation

```liquid
{% if attempt > 2 %}
This issue has failed {{ attempt }} times. Consider simplifying the approach or adding more debug output.
{% endif %}
```

## Extending the Tracker

The tracker interface is trait-based:

```rust
#[async_trait]
pub trait IssueTracker: Send + Sync {
    async fn fetch_candidate_issues(&self) -> Result<Vec<Issue>, SympheoError>;
    async fn fetch_issues_by_states(&self, states: &[String]) -> Result<Vec<Issue>, SympheoError>;
    async fn fetch_issue_states_by_ids(&self, ids: &[String]) -> Result<Vec<Issue>, SympheoError>;
}
```

To add a new tracker (e.g., Linear, Jira):

1. Implement the `IssueTracker` trait in a new module under `src/tracker/`.
2. Add a new variant to the tracker factory in `main.rs`.
3. Update the configuration schema in `src/config/typed.rs` to accept the new tracker type.

## Extending the Backend

Similarly, the agent backend is trait-based:

```rust
#[async_trait]
pub trait AgentBackend: Send + Sync {
    async fn run_turn(...);
    async fn cleanup_workspace(...);
}
```

You can implement a new backend for a different agent platform (e.g., Claude Code, Aider) by implementing `AgentBackend` and wiring it into `AgentRunner`.

## Backup and Disaster Recovery

Sympheo's scheduler state is intentionally in-memory (SPEC §14.3). On restart, the service rebuilds useful state by:

1. **Startup terminal cleanup** — querying the tracker for terminal-state issues and removing the corresponding workspaces.
2. **Fresh polling** — fetching active issues and re-dispatching eligible work.

What survives a restart:

- Workspaces on disk (reused on the next dispatch for the same issue).
- Tracker state (the source of truth).

What does NOT survive:

- Retry timers.
- Live agent sessions (in-flight subprocess turns).
- In-memory token totals.

**Subprocess hygiene** — the local backend keeps a process registry of every CLI subprocess it spawns and signals each process group on shutdown. After a clean `SIGINT`, no zombie agent is left behind. Crashes (panics, kills) leave the registry intact for the next process to find — but cross-process cleanup is not implemented.

To minimize orphan work, drain active issues before restarting (move them to a temporary paused column on the board) or accept that the next tick will re-dispatch them.

## Trust Boundary

Sympheo does not validate agent outputs (SPEC §2.2, §11.5, §15.1). The trust boundary explicitly extends to the agent: if the agent transitions a ticket without producing the expected artifacts, Sympheo will not detect it. Workflows that need such guarantees must encode them in the agent prompt, in workflow hooks, or in downstream review processes.

The local backend isolates the per-worker filesystem-config under `<workspace>/.sympheo-home/`:

- `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME` are mapped into the workspace.
- `PATH` defaults to `<HOME>/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`.
- Inherited env is cleared except for a small whitelist: `LANG`, `LANGUAGE`, `LC_*`, `TERM`, `TZ`, `USER`, `LOGNAME`.
- Operator overrides via `cli.env` always win (last layer).

This prevents cross-worker contamination, prevents the agent from reading the host operator's `~/.config/opencode` or similar, and prevents credential-shaped host env vars (`ANTHROPIC_API_KEY`, `AWS_ACCESS_KEY_ID`, `GITHUB_TOKEN`) from leaking unless explicitly listed in `cli.env`. It does **not** prevent absolute-path filesystem reads (`cat /etc/passwd`) or constrain network — for that, use the Daytona backend or run Sympheo inside a container.

## Security Checklist

Before running Sympheo in a shared or production environment:

- [ ] Decide your **trust posture** explicitly (SPEC §15.1) — high-trust (current default with `--dangerously-skip-permissions`) or restricted.
- [ ] Use the Daytona backend for OS-level agent isolation if the local HOME/XDG scoping is not enough.
- [ ] Scope the GitHub token to the minimum required permissions (`repo`, `project`).
- [ ] Rotate `GITHUB_TOKEN` regularly.
- [ ] Do not commit tokens or `WORKFLOW.md` files containing secrets to version control.
- [ ] Run Sympheo behind a firewall; the dashboard has no authentication.
- [ ] Review every skill file for instructions that could lead to data exfiltration (e.g., "upload this file to an external server").
- [ ] Restrict the workspace root to a dedicated volume or directory; Sympheo enforces path containment but does not enforce filesystem-wide jailing.
- [ ] Audit `cli.env` overrides: any variable you add bypasses the default isolation and is exposed to the agent.

## Internal Error Categories

When debugging via logs or the retry queue's `error` field, the underlying error follows the SPEC §10.5 / §11.3 categories:

| Category | Source | Typical cause |
|---|---|---|
| `cli_adapter_not_found` | dispatch validation | `cli.command` leading binary doesn't match any registered adapter (`opencode`, `pi`, `mock-cli`). |
| `cli_not_found` | CLI adapter `validate` | The binary is not on `PATH`. |
| `invalid_workspace_cwd` | local backend | The cwd does not match the per-issue workspace path; safety invariant violated. |
| `session_start_failed` | adapter `start_session` | One-time setup failed (e.g. auth missing). |
| `turn_launch_failed` | adapter `run_turn` | Subprocess failed to spawn. |
| `turn_read_timeout` / `turn_total_timeout` | adapter `run_turn` | `cli.read_timeout_ms` / `cli.turn_timeout_ms` exceeded. |
| `turn_cancelled` | operator | Cancellation via `POST /api/v1/<id>/cancel`. |
| `turn_failed` | adapter `run_turn` | Agent returned an explicit failure or rate-limit. |
| `subprocess_exit` | adapter | Non-zero exit code from the CLI. |
| `output_parse_error` | adapter parser | CLI emitted a malformed event. |
| `user_input_required` | adapter | Agent requested interactive input; treated as a failure under high-trust posture. |
| `tracker_request_failed` / `tracker_status_error` / `tracker_graphql_errors` | tracker adapter | Transport, HTTP or GraphQL error from GitHub. |
| `missing_tracker_auth` / `missing_tracker_project_identity` | tracker validate | `api_key` or `project_slug`/`project_number` is empty after `$VAR` resolution. |
