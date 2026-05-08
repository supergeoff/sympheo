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

Sympheo itself does not persist state to disk. All state is held in memory and rebuilt from the GitHub board on every poll. This means:

- **Restarting Sympheo is safe** — it will rediscover all active issues and resume work.
- **Running agents may be orphaned** — if Sympheo restarts while agents are running, it will lose track of those sessions. The agents will continue until they finish or timeout, but Sympheo will start new turns for the same issues on the next tick.

To minimize orphan agents, avoid restarting Sympheo during busy periods, or move active issues to a temporary "paused" column before restarting.

## Security Checklist

Before running Sympheo in a shared or production environment:

- [ ] Use Daytona mode for agent isolation.
- [ ] Scope the GitHub token to the minimum required permissions.
- [ ] Rotate `GITHUB_TOKEN` regularly.
- [ ] Do not commit tokens or `WORKFLOW.md` files containing secrets to version control.
- [ ] Run Sympheo behind a firewall; the dashboard does not have authentication.
- [ ] Review skill files for instructions that could lead to data exfiltration (e.g., "upload this file to an external server").
