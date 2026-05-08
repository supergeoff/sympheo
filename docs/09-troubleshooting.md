# Troubleshooting

This section covers common problems and how to resolve them.

## Startup Issues

### "startup validation failed"

Sympheo exits immediately with a validation error. Check the log output for the exact field that failed.

**Common causes:**
- Missing `GITHUB_TOKEN` environment variable (or whichever variable is referenced in the config).
- `workspace.root` path does not exist or is not writable.
- `skills.mapping` points to a skill file that does not exist.
- `daytona.enabled: true` but `daytona.api_key` or `daytona.server_url` is missing.

**Fix:**
```bash
export GITHUB_TOKEN=ghp_xxx
cargo run
```

### "failed to load workflow"

The `WORKFLOW.md` file is malformed.

**Common causes:**
- Missing `---` delimiters around the YAML block.
- Invalid YAML syntax (indentation errors, tabs instead of spaces).
- The Liquid template contains unclosed tags like `{% if ... %}` without `{% endif %}`.

**Fix:** Validate the YAML block with an online YAML parser or run:
```bash
cat WORKFLOW.md | sed -n '/---/,/---/p' | head -n -1 | tail -n +2 | yq '.'
```

## Agent Issues

### "Sympheo isn't picking up my issues"

1. Verify the issue is in a column listed in `tracker.active_states`.
2. Check that the issue is not blocked by another active issue.
3. Ensure `max_concurrent_agents` has not been reached.
4. Trigger a manual refresh and watch the logs:
   ```bash
   curl -X POST http://localhost:9090/api/v1/refresh
   ```

### "Agent fails immediately"

1. Check the agent command exists and is in your `PATH`:
   ```bash
   which opencode
   ```
2. Verify the `codex.command` in your config is correct.
3. Check the `after_create` hook output. If the repo failed to clone, the agent has no code to work on.
4. Look at the retry queue in the dashboard for the error message.

### "Agent stalls or times out"

The dashboard shows an agent stuck with no new events.

**Causes:**
- The agent is waiting for interactive input (it should not, but misconfigured agents might).
- The build or test suite is taking longer than `codex.turn_timeout_ms`.
- The agent entered an infinite loop.

**Fixes:**
- Increase `codex.stall_timeout_ms` or `codex.turn_timeout_ms` in `WORKFLOW.md`.
- Add logging to your `before_run` hook to confirm the environment is correct.
- Cancel the issue by moving it to a terminal state on the board.

### "Agent uses too many tokens"

- Reduce the length of your base prompt template.
- Move verbose instructions into skills only for the stages that need them.
- Lower `agent.max_turns` to prevent runaway retries.

## Workspace Issues

### "Workspace not cleaning up"

1. Verify the issue is in a `terminal_state`.
2. Check the `before_remove` hook is not hanging (it must complete within `hooks.timeout_ms`).
3. If using Daytona mode, verify the Daytona API is reachable.

### "Disk space filling up"

Local workspaces accumulate under `workspace.root`. Sympheo only cleans workspaces for issues it knows about. If you delete issues from GitHub directly, Sympheo may not clean their workspaces.

**Fix:** Manually prune old workspace directories:
```bash
rm -rf ~/sympheo_workspaces/*
```

## Tracker Issues

### "GitHub API rate limit errors"

If you see rate limit errors in the logs:
- Increase `polling.interval_ms` to reduce request frequency.
- Use a GitHub token with higher rate limits (GitHub Apps have higher limits than personal tokens).
- The dashboard exposes rate limit state under `rate_limits` in the `/api/v1/state` response.

### "Project not found"

- Double-check `project_slug` (format must be `owner/repo`).
- Ensure `project_number` matches the number in your GitHub Project URL.
- Verify your GitHub token has access to the repository and project.

## Dashboard Issues

### "Dashboard shows 404"

- Confirm `server.port` is set in `WORKFLOW.md` or `--port` was passed on the CLI.
- Check the logs for "HTTP server listening" to see the actual bound port.
- Ensure no firewall is blocking localhost traffic.

### "Dashboard data is stale"

The dashboard refreshes every 5 seconds via JavaScript. If it looks stale:
- Hard-refresh the browser.
- Check if the orchestrator tick loop is still running (look for recent log lines).
- Call `/api/v1/refresh` to force a poll.

## FAQ

**Q: Can I use a different issue tracker (Jira, Linear, etc.)?**

A: Not out of the box, but the tracker is defined as a trait in the codebase. You would need to implement the `IssueTracker` trait for your platform.

**Q: Can I run Sympheo in Docker?**

A: Yes. Build a Docker image with Rust, copy your `WORKFLOW.md` and skills into the image, and run `cargo run`. Just ensure the container has access to the required environment variables.

**Q: Can multiple Sympheo instances use the same GitHub Project?**

A: Technically yes, but they will race for the same issues. It is safer to partition by labels or use different project views.

**Q: How do I stop Sympheo gracefully?**

A: Send `SIGINT` (Ctrl+C). Sympheo will finish the current tick and then exit. Running agents may be orphaned depending on your backend.
