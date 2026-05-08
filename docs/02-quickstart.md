# Quick Start

Get Sympheo running against your GitHub Project in under 10 minutes.

## Prerequisites

- [Rust](https://rustup.rs/) installed (1.75+ recommended)
- A [GitHub personal access token](https://github.com/settings/tokens) with `repo` and `project` scopes
- A GitHub repository with a [GitHub Project](https://docs.github.com/en/issues/planning-and-tracking-with-projects/creating-projects) board

## 1. Clone the Repository

```bash
git clone https://github.com/supergeoff/sympheo.git
cd sympheo
```

## 2. Create Your Workflow File

Copy the example below into a file named `WORKFLOW.md` at the project root. Replace the placeholders with your own values.

```yaml
---
tracker:
  kind: github
  api_key: $GITHUB_TOKEN
  project_slug: your-org/your-repo
  project_number: 1
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

polling:
  interval_ms: 30000

workspace:
  root: ~/sympheo_workspaces

hooks:
  after_create: |
    git clone https://x-access-token:${GITHUB_TOKEN}@github.com/your-org/your-repo.git .
  before_run: |
    echo "Starting run for issue"
  after_run: |
    echo "Run finished"
  before_remove: |
    echo "Removing workspace"
  timeout_ms: 60000

agent:
  max_concurrent_agents: 5
  max_turns: 10
  max_retry_backoff_ms: 300000

codex:
  command: opencode run
  turn_timeout_ms: 3600000
  read_timeout_ms: 5000
  stall_timeout_ms: 300000

server:
  port: 9090

skills:
  mapping:
    spec: ./skills/spec/SKILL.md
    in progress: ./skills/build/SKILL.md
    review: ./skills/review/SKILL.md
    test: ./skills/test/SKILL.md
    doc: ./skills/doc/SKILL.md
---

You are working on issue {{ issue.identifier }}: {{ issue.title }}.

Description: {{ issue.description }}

The repository has been cloned into this workspace. Please analyze the issue, implement the necessary changes, and ensure tests pass.

The project board has the following columns and workflow:
- **Todo**: Issues waiting to be picked up.
- **Spec**: Technical specification and design phase. When ready to implement, move to "In Progress".
- **In Progress**: Active development and implementation phase. Work in a dedicated branch. When done, move to "Review".
- **Review**: Code review phase. When review passes, move to "Test".
- **Test**: Testing and validation phase. When tests pass, move to "Doc".
- **Doc**: Documentation phase. When documentation is complete, open a PR and move to "Done".
- **Done**: Completed and verified. This is the terminal state.

When you finish the work for the current column, move this issue to the next appropriate column using `gh` or the GitHub API with the `GITHUB_TOKEN` environment variable.

**IMPORTANT:** Do NOT run `sympheo`, `cargo run`, or any orchestrator commands. Do NOT start long-running background processes or servers. Focus only on implementing the requested changes.

{% if attempt %}This is retry/continuation attempt {{ attempt }}.{% endif %}
```

> **Note:** The `---` delimiters separate the YAML configuration from the Liquid prompt template. The template is what the agent sees at the top of every conversation.

## 3. Set Environment Variables

```bash
export GITHUB_TOKEN=ghp_your_token_here
```

## 4. Build and Run

```bash
cargo run
```

If you want the web dashboard on a specific port:

```bash
cargo run -- --port 9090
```

Or pass a custom workflow file:

```bash
cargo run -- /path/to/my/WORKFLOW.md
```

## 5. Verify It's Working

1. Open your browser to [http://localhost:9090](http://localhost:9090) (or whatever port you configured).
2. You should see the **Sympheo Dashboard** with running sessions, retry queues, and token usage.
3. Create a test issue in your GitHub Project and move it to the **Todo** column.
4. Within one polling interval (30 seconds by default), Sympheo should pick it up and start a workspace.
5. Watch the dashboard for the new session to appear.

## Next Steps

- Learn about the [core concepts](03-core-concepts.md) to understand the full lifecycle.
- Read the [configuration reference](04-configuration.md) to tune every setting.
- Explore [skills](06-skills.md) to customize agent behavior for each stage.
