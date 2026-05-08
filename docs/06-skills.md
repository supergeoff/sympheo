# Skills

A **skill** is a specialized prompt that shapes how the agent behaves when working on an issue in a particular workflow stage. Think of it as a role card: "You are an Architect," "You are a Tech Lead," "You are a Code Reviewer."

## How Skills Work

When Sympheo dispatches an agent for an issue, it builds the final prompt in this order:

1. **Base prompt template** — from `WORKFLOW.md` (the Liquid template after the `---` block).
2. **Skill prompt** — appended if the issue's current state has a mapped skill.
3. **Rendered variables** — Liquid placeholders like `{{ issue.title }}` are substituted with real values.

The combined prompt is passed to the agent command (e.g., `opencode run`) via stdin or however the backend protocol works.

## Built-In Skills

Sympheo ships with five default skills covering the full delivery lifecycle:

### `architect-spec` (Spec stage)

The agent acts as a Senior Architect. It explores the codebase thoroughly, enforces architectural consistency, and produces a comprehensive Low-Level Design (LLD) that becomes the single source of truth for implementation.

Key directives:
- Do NOT write implementation code.
- Output a complete LLD with component design, data models, error handling, and testing requirements.

### `techlead-build` (In Progress stage)

The agent acts as a Senior Tech Lead. It takes the LLD from the Spec stage and implements it with strict Test-Driven Development discipline.

Key directives:
- Write failing tests before implementation (red-red-green).
- Match existing code style exactly.
- Run `cargo test`, `cargo clippy`, and `cargo fmt` before finishing.

### `code-reviewer-review` (Review stage)

The agent acts as an Expert Code Reviewer. It validates the implementation against the LLD and judges correctness, elegance, and consistency.

Key directives:
- Produce a structured review report: `APPROVED`, `APPROVED_WITH_COMMENTS`, or `CHANGES_REQUESTED`.
- Blocking issues must be resolved before approval.

### `test-expert-test` (Test stage)

The agent acts as a Testing Expert. It audits existing test coverage, writes missing tests, and ensures all user journeys are covered.

Key directives:
- Target ≥80% line coverage on unit tests.
- Target 100% coverage of user journeys via E2E tests.
- All tests must pass before the stage is complete.

### `doc-expert-doc` (Doc stage)

The agent acts as a Documentation Expert. It updates all documentation to reflect the current state of the codebase.

Key directives:
- Update README, inline doc comments, changelogs, and architecture docs.
- Run `cargo test --doc` and `cargo doc` to verify examples.

## Writing a Custom Skill

A skill is just a Markdown file with a YAML front matter block followed by instructions.

### File Structure

```markdown
---
name: my-custom-skill
description: What this skill does and when it triggers
---

# Skill: My Custom Skill

You are a [role]. Your mission is to [objective].

## Identity

- Trait 1
- Trait 2

## Input

What the agent receives.

## Output

What the agent must produce.

## Process

### 1. Step One
Instructions...

### 2. Step Two
Instructions...

## Rules

- Rule one.
- Rule two.
```

### Example: Security Review Skill

```markdown
---
name: security-review
description: Runs a security-focused review before code reaches production
---

# Skill: Security Reviewer

You are a Security-Focused Code Reviewer. Your mission is to identify vulnerabilities, unsafe patterns, and credential leaks in the proposed changes.

## Identity

- Paranoid about user input and network boundaries.
- Expert in Rust security (unsafe blocks, panic paths, deserialization).

## Process

1. Read the diff or modified files.
2. Check for:
   - Unsanitized user input reaching system calls or SQL.
   - Hardcoded secrets or tokens.
   - Unsafe Rust without documented invariants.
   - Panic paths in async contexts.
3. Produce a security report.

## Rules

- Any finding rated CRITICAL blocks approval.
- SUGGESTION-level findings are optional but appreciated.
```

Save this as `skills/security/SKILL.md` and add it to your config:

```yaml
skills:
  mapping:
    security: ./skills/security/SKILL.md
```

## Skill Best Practices

1. **Be explicit about output format** — Agents parse instructions literally. If you want a Markdown report, say so.
2. **Limit scope** — A skill should cover one stage of work. Don't ask the agent to both design and implement in the same skill.
3. **Reference tools** — If the agent should run `cargo test` or `gh pr create`, include the exact command.
4. **Use constraints, not vague advice** — "Write tests first" is better than "Consider testing."
5. **Keep it under the context window** — Very long skills consume token budget. If a skill exceeds a few thousand tokens, split it or link to external docs.
6. **Version your skills** — Track skill changes in Git. A skill is part of your team's process definition.

## Skill Mapping Reference

| Config Key | File Path | Stage |
|------------|-----------|-------|
| `spec` | `./skills/spec/SKILL.md` | Design / specification |
| `in progress` | `./skills/build/SKILL.md` | Implementation |
| `review` | `./skills/review/SKILL.md` | Code review |
| `test` | `./skills/test/SKILL.md` | Testing |
| `doc` | `./skills/doc/SKILL.md` | Documentation |
