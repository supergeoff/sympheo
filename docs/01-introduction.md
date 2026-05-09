# Introduction

Sympheo is an autonomous orchestrator that turns your GitHub project board into a self-driving development pipeline. It watches for issues, dispatches AI coding agents to do the work, and advances tickets through your workflow automatically.

## What Problem Does It Solve?

Modern development teams manage work through project boards, but moving tickets from "todo" to "done" still requires human effort: writing specs, implementing features, reviewing code, writing tests, and updating documentation. Sympheo automates this entire lifecycle by pairing each board column with a specialized AI agent that knows exactly how to perform that stage of work.

## How It Works at a Glance

```
GitHub Project Board
       │
       ▼
┌──────────────┐
│   Sympheo    │  ← polls for issues in active states
│ Orchestrator │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Workspace  │  ← cloned repo + isolated environment
│  (local or   │
│   Daytona)   │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│    Agent     │  ← opencode / Codex with stage-specific skill
│   (Skill)    │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Result     │  ← code, tests, docs, PR
└──────────────┘
```

1. **Track** — Sympheo polls your GitHub Project for issues in active columns.
2. **Prepare** — For each issue, it creates an isolated workspace and clones the repository.
3. **Dispatch** — It launches a coding agent with a contextual prompt and a stage-specific skill.
4. **Stream** — It monitors the agent's progress in real time, tracking tokens and detecting stalls.
5. **Advance** — When the agent finishes, the issue moves to the next column and the cycle repeats.
6. **Clean up** — When an issue reaches a terminal state, the workspace is removed.

## Key Concepts

| Term | Meaning |
|------|---------|
| **Tracker** | The issue source. Currently GitHub Projects. |
| **Workflow** | The state machine mapped to your project board columns. |
| **Skill** | A specialized prompt file that tells the agent how to behave for a specific stage. |
| **Workspace** | An isolated directory (local) or Daytona sandbox where the agent works. |
| **Turn** | A single execution of the agent against an issue. Multiple turns may run if retries are needed. |
| **Hook** | A shell script triggered at lifecycle events (workspace creation, agent start, cleanup). |
| **Orchestrator** | The core engine that polls, reconciles, dispatches, and monitors. |

## When to Use Sympheo

- You manage work through a GitHub Project board with clear stage columns.
- You want AI agents to handle spec writing, implementation, review, testing, and documentation.
- You need isolation between tasks (each issue gets its own workspace).
- You want retries, monitoring, and automatic cleanup out of the box.

## When Not to Use Sympheo

- Your workflow is highly unstructured or changes frequently.
- You need human approval gates that cannot be automated.
- Your repository requires secrets or credentials that cannot be provided to an agent safely.

## What Sympheo Does Not Do (Trust Boundary)

These are explicit non-goals from the [normative specification](../SPEC.md) (§2.2 / §11.5 / §15.1). Read them before relying on Sympheo in production:

- **Sympheo does not transition tickets.** State changes (`Todo → Spec → … → Done`) are performed by the agent itself, using `gh`, the GitHub API, or another tracker tool exposed through the workflow prompt.
- **Sympheo does not validate the agent's output.** It does not check that a PR was opened, that a branch exists, that tests pass, or that the body of an issue was updated before the agent advances the column. Guarantees of that kind are the job of your workflow prompt, your skills, and your downstream review process.
- **Sympheo does not enforce a sandbox by default.** The local backend isolates the agent's filesystem-config (HOME, XDG, PATH) per worker, but it does not prevent absolute path access (`cat /etc/passwd`) or constrain network. Use the Daytona backend or run Sympheo inside a container for stronger isolation.
- **Sympheo does not persist scheduler state.** Retry timers and live worker state are in-memory only. Restarts rebuild from the tracker; in-flight sessions are not recoverable.

If you need any of these, you must encode them in the workflow prompt, in hooks, in skills, or in downstream review — not by expecting the orchestrator to enforce them.
