# Workflow States

Sympheo models your development process as a state machine where each state maps to a column on your GitHub Project board. The built-in workflow is designed around a classic software delivery pipeline, but you can adapt it to your own board.

## Built-In State Machine

The default workflow assumes the following columns on your GitHub Project:

| State | Purpose | Skill Injected |
|-------|---------|----------------|
| **Todo** | Backlog / ready to start | None (base prompt only) |
| **Spec** | Technical design and specification | `architect-spec` |
| **In Progress** | Implementation and coding | `techlead-build` |
| **Review** | Code review and quality gate | `code-reviewer-review` |
| **Test** | Test coverage validation | `test-expert-test` |
| **Doc** | Documentation and changelog | `doc-expert-doc` |
| **Done** | Completed and verified | Terminal state — no agent |

## How States Drive Behavior

### Active States

Any state listed in `tracker.active_states` is polled continuously. When an issue is found in one of these states:

1. Sympheo ensures a workspace exists.
2. If the state has a skill mapping, the skill content is appended to the prompt template.
3. The agent is launched (or a new turn is started if already running).

### Terminal States

Any state listed in `tracker.terminal_states` signals that work is complete:

- The agent is stopped if still running.
- The workspace is cleaned up (`before_remove` hook runs, then deletion).
- The issue is removed from the orchestrator's internal tracking.

### State Transitions

Sympheo itself does **not** move issues on the board. The agent is expected to do this via the GitHub CLI (`gh`) or GitHub API, using the `GITHUB_TOKEN` provided in its environment.

A typical transition flow looks like this:

1. Human moves issue to **Todo**.
2. Sympheo dispatches agent with base prompt. Agent analyzes and moves issue to **Spec**.
3. Sympheo detects the state change, injects `architect-spec` skill, and dispatches a new turn.
4. Agent writes the LLD and moves issue to **In Progress**.
5. Sympheo injects `techlead-build` skill. Agent implements the feature and moves issue to **Review**.
6. And so on until **Done**.

## Customizing States

Your GitHub Project does not need to use the exact same column names. You can map any column names to skills and state lists:

```yaml
tracker:
  active_states:
    - backlog
    - design
    - dev
    - qa
    - ship
  terminal_states:
    - shipped
    - wontfix

skills:
  mapping:
    design: ./skills/spec/SKILL.md
    dev: ./skills/build/SKILL.md
    qa: ./skills/test/SKILL.md
    ship: ./skills/doc/SKILL.md
```

> **Important:** State names in `skills.mapping` must match the names in `active_states` exactly (case-insensitive matching is used internally, but consistency avoids confusion).

## Blocked Issues

If an issue has linked dependencies that are still in active states, Sympheo will skip it. The dashboard will not show it as running, and no workspace will be created until all blockers reach terminal states.

To unblock an issue, move its dependencies to a terminal state on the GitHub board.
