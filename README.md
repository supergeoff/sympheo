# Sympheo

> An autonomous orchestrator that turns your GitHub project board into a self-driving development pipeline.

Sympheo watches your project board, dispatches AI coding agents to handle the work, and advances tickets through your workflow automatically. It is the bridge between your issue tracker and autonomous code agents.

## What It Does

- **Tracks** your GitHub Project for issues in active columns (todo, spec, in progress, review, test, doc).
- **Prepares** an isolated workspace for each issue — locally or inside a Daytona sandbox.
- **Dispatches** a specialized agent for the issue's current stage, injecting stage-specific skills (Architect, Tech Lead, Code Reviewer, Test Expert, Doc Expert).
- **Monitors** agent progress in real time, tracking token usage and detecting stalls or timeouts.
- **Cleans up** workspaces automatically when issues reach terminal states (done, closed, cancelled).

## Why Sympheo?

Moving tickets from "todo" to "done" still requires human effort: writing specs, coding, reviewing, testing, documenting. Sympheo automates this lifecycle by pairing each board column with an AI agent that knows exactly how to perform that stage of work.

## Quick Teaser

```bash
# 1. Configure your tracker, workflow, and skills in WORKFLOW.md
# 2. Set your token
export GITHUB_TOKEN=ghp_xxx

# 3. Run
cargo run

# 4. Open the dashboard
open http://localhost:9090
```

Move an issue on your GitHub board. Sympheo picks it up within seconds.

## Local Development Setup

After cloning:

```bash
git config core.hooksPath .githooks
```

This activates local pre-commit and commit-msg hooks enforcing:

**Bypass locally**: `git commit --no-verify`

## Documentation

All user documentation lives in the [`docs/`](docs/) directory:

| Document | What you'll learn |
|----------|-----------------|
| [`01-introduction.md`](docs/01-introduction.md) | What Sympheo is, key concepts, when to use it |
| [`02-quickstart.md`](docs/02-quickstart.md) | Prerequisites, full setup, and first run |
| [`03-core-concepts.md`](docs/03-core-concepts.md) | How the issue lifecycle, polling, and workspaces work |
| [`04-configuration.md`](docs/04-configuration.md) | Complete `WORKFLOW.md` reference |
| [`05-workflow-states.md`](docs/05-workflow-states.md) | The state machine and how to customize it |
| [`06-skills.md`](docs/06-skills.md) | How skills work and how to write your own |
| [`07-execution-modes.md`](docs/07-execution-modes.md) | Local vs Daytona execution |
| [`08-monitoring.md`](docs/08-monitoring.md) | Dashboard, REST API, logs, and metrics |
| [`09-troubleshooting.md`](docs/09-troubleshooting.md) | Common issues and FAQ |
| [`10-advanced-topics.md`](docs/10-advanced-topics.md) | CI hooks, multi-instance setup, custom backends |

## Development

```bash
cargo test
cargo run
```

## License

MIT
