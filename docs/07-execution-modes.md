# Execution Modes

Sympheo supports two ways to run agents: **locally on the host machine** or inside **Daytona sandboxes**. You choose the mode via the `daytona` configuration block.

## Local Mode

By default, if the `daytona` block is absent or `daytona.enabled` is `false`, Sympheo runs agents directly on the machine where Sympheo is started.

### How It Works

- Sympheo creates a directory under `workspace.root` for each issue (e.g., `~/sympheo_workspaces/ISSUE-42`).
- The `after_create` hook is responsible for cloning the repository and setting up the environment.
- The agent process (`codex.command`) is spawned as a child process on the host.
- Sympheo reads the agent's stdout and parses JSON events from the stream.
- The agent's **stderr is captured and logged** at `WARN` level, tagged with the issue ID, so diagnostic output is visible in the orchestrator logs.

### When to Use Local Mode

- You are developing or testing Sympheo itself.
- Your repository is small and builds quickly.
- You want minimal overhead and no external dependencies.
- You trust the agent to run on the same machine as your source code.

### Security Considerations

In local mode, the agent has the same filesystem access as the user running Sympheo. Use the `before_run` hook to restrict access if needed, or prefer Daytona for untrusted code generation.

## Daytona Mode

Daytona is a platform for provisioning isolated development environments. When enabled, Sympheo creates a Daytona sandbox for each issue instead of a local directory.

### Configuration

```yaml
daytona:
  enabled: true
  api_key: $DAYTONA_API_KEY
  server_url: https://api.daytona.io
  target: local
```

| Field | Description |
|-------|-------------|
| `enabled` | Must be `true` to activate Daytona mode. |
| `api_key` | Your Daytona API key. Supports `$ENV_VAR` interpolation. |
| `server_url` | The Daytona control plane URL. |
| `target` | The Daytona target to use (e.g., `local`, `aws`, `gcp`). |

### How It Works

- On the first turn for an issue, Sympheo calls the Daytona API to create a sandbox.
- The workspace path becomes a reference to the sandbox rather than a local directory.
- The agent runs inside the sandbox. All file I/O and process execution happens in isolation.
- When the issue reaches a terminal state, Sympheo destroys the sandbox via the Daytona API.

### When to Use Daytona Mode

- You need strong isolation between issues.
- Your build process requires specific OS dependencies or custom images.
- You run Sympheo on a shared server and cannot trust agents with host filesystem access.
- You want to scale horizontally by offloading compute to Daytona targets.

### Hybrid Scenarios

While Sympheo does not currently support per-issue selection out of the box, you can approximate hybrid behavior by running two Sympheo instances:

- One instance configured for local execution (small issues, trusted changes).
- One instance configured for Daytona execution (large refactors, security-sensitive work).

Use GitHub labels to route issues to the appropriate instance by filtering `active_states` or using project views.

## Comparing Modes

| Aspect | Local | Daytona |
|--------|-------|---------|
| Setup complexity | Low | Requires Daytona account and API key |
| Isolation | Process-level | Full sandbox |
| Startup time | Instant | Seconds (sandbox creation) |
| Resource usage | Host CPU/memory | Offloaded to Daytona target |
| Cleanup | Directory deletion | API call to destroy sandbox |
| Best for | Development, trusted agents | Production, shared infrastructure |

## Switching Modes

You can change modes by editing `WORKFLOW.md` and letting Sympheo hot-reload the config. Note that existing running agents will continue on their original backend; only newly dispatched agents will use the new mode.
