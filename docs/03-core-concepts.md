# Core Concepts

Understanding these concepts will help you reason about how Sympheo behaves and how to configure it effectively.

## The Issue Lifecycle

An issue moves through a predictable lifecycle from the moment it appears on your board to the moment it is archived.

```
GitHub Project
      │
      ▼
┌─────────────┐
│    Todo     │ ← active state: Sympheo picks it up
└──────┬──────┘
       │
       ▼
┌─────────────┐
│    Spec     │ ← skill: architect-spec
└──────┬──────┘
       │
       ▼
┌─────────────┐
│ In Progress │ ← skill: techlead-build
└──────┬──────┘
       │
       ▼
┌─────────────┐
│   Review    │ ← skill: code-reviewer-review
└──────┬──────┘
       │
       ▼
┌─────────────┐
│    Test     │ ← skill: test-expert-test
└──────┬──────┘
       │
       ▼
┌─────────────┐
│    Doc      │ ← skill: doc-expert-doc
└──────┬──────┘
       │
       ▼
┌─────────────┐
│    Done     │ ← terminal state: workspace cleaned up
└─────────────┘
```

1. **Detection** — During each polling tick, Sympheo fetches all issues in `active_states`.
2. **Filtering** — Blocked issues (those with open dependencies) are skipped until their blockers are resolved.
3. **Workspace creation** — A fresh workspace is prepared. In local mode this is a directory; in Daytona mode it is a sandbox.
4. **Agent dispatch** — The agent is launched with the base prompt template plus any skill mapped to the issue's current state.
5. **Streaming** — Sympheo monitors the agent's output stream, tracking tokens, events, and detecting stalls or timeouts.
6. **Completion / retry** — If the turn succeeds, the issue is eligible to move forward. If it fails, Sympheo queues a retry with exponential backoff.
7. **Terminal cleanup** — When an issue reaches a `terminal_state`, its workspace is destroyed and the issue is no longer tracked.

## Polling Loop

Sympheo runs an internal timer that ticks every `polling.interval_ms` (default: 30 seconds). On each tick:

1. Fetch candidate issues from the tracker.
2. Reconcile internal state with the fetched list.
3. Start new agents for newly detected issues.
4. Check for stalled or timed-out agents.
5. Process the retry queue.

You can trigger an immediate extra poll by calling the `/api/v1/refresh` endpoint.

## Turns and Sessions

A **turn** is a single invocation of the agent command for a given issue. One issue may go through many turns:

- **First turn** — The initial dispatch when the issue enters an active state.
- **Retry turn** — If the agent exits with an error or times out, Sympheo schedules a retry.
- **Continuation turn** — If the agent reports that it is not finished (e.g., needs to move to the next column), Sympheo may dispatch another turn depending on your configuration.

Each turn creates a **session** that Sympheo tracks in memory. The dashboard shows live session data including turn count, tokens consumed, and the last event received.

## Workspaces

Every issue gets its own workspace. This guarantees isolation: dependencies, build artifacts, and file changes from one issue never leak into another.

**Local workspaces** are directories under `workspace.root` (e.g., `~/sympheo_workspaces/ISSUE-42`).

**Daytona workspaces** are ephemeral sandboxes managed by the Daytona platform. Sympheo creates them on demand and destroys them after use.

The `after_create` hook is the right place to clone your repository or install dependencies into a fresh workspace.

## Blockers

If an issue has linked "blocked by" relationships in GitHub, Sympheo checks whether those blockers are in terminal states. If a blocker is still active, the issue is skipped until the blocker is resolved. This prevents agents from working on features whose prerequisites are incomplete.

## Reconciliation

Sympheo maintains an internal view of the world. On every tick it reconciles this view with the actual state of the GitHub board:

- Issues moved into active states are discovered and queued.
- Issues moved into terminal states are cleaned up.
- Issues whose state changed on the board are updated in the orchestrator.
- Running agents whose issues disappeared from the board are cancelled.

## Hot Reload

Sympheo watches `WORKFLOW.md` for changes. If you edit the file while the orchestrator is running, it will reload the configuration and skill mappings automatically without requiring a restart. This is useful for tuning timeouts or adjusting skill paths on the fly.
