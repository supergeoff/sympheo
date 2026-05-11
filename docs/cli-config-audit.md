# CLI config — shared contract across adapters

`cli.*` in `WORKFLOW.md` is the operator-facing config surface that drives
every coding-agent CLI Sympheo can spawn (`opencode`, `claude`, `pi`, plus
the test-only `mock-cli`). The contract is identical across adapters: the
same typed schema lands in the same place, and each adapter projects the
typed fields onto its own native flag set.

## Top-level `cli.*` fields

Source of truth: `src/config/typed.rs` (`cli_*` accessors) and
`src/agent/cli/mod.rs` (`CliConfig`, `CliOptions`).

| Field                   | Default      | Consumer                                                                     |
|-------------------------|--------------|-------------------------------------------------------------------------------|
| `cli.command`           | (required)   | adapter `validate()` + `build_command_string()`; spawned by `LocalBackend` via `bash -lc` |
| `cli.env`               | `{}`         | `LocalBackend` merges into subprocess env. `MockBackend` ignores.             |
| `cli.options`           | `{}`         | typed triplet (`model`, `permission`, `additional_args`) — see below           |
| `cli.turn_timeout_ms`   | `3600000`    | `LocalBackend` turn-wide timeout                                              |
| `cli.read_timeout_ms`   | `5000`       | `LocalBackend` per-stdout-line timeout                                        |
| `cli.stall_timeout_ms`  | `300000`     | `LocalBackend` stall detection                                                |

`validate_for_dispatch` rejects any `cli.*` field outside this surface; in
particular, `cli.args` is no longer accepted — its replacement is
`cli.options.additional_args`.

## `cli.options` — the shared typed triplet

The single typed view consumed by every production adapter
(`src/agent/cli/mod.rs::CliOptions`):

| Key              | Type                                  | Meaning                                                                 |
|------------------|---------------------------------------|-------------------------------------------------------------------------|
| `model`          | `string?`                             | Native model identifier passed via the adapter's `--model` flag         |
| `permission`     | enum `plan` \| `acceptEdits` \| `bypassPermissions` \| `default` | Agent permission mode (see per-adapter projection)        |
| `additional_args`| `string[]`                            | Verbatim shell tokens appended to the assembled argv, shell-escaped per token, with `$VAR` resolution |

The parser hard-rejects three legacy keys with a rename-pointing error:

- `permission_mode` → `permission`
- `permissions` (plural) → `permission` (singular)
- `mcp_servers` → no longer supported (declare MCP servers via the agent's
  own config file)

Unknown keys are silently ignored from the typed view but remain accessible
through `ServiceConfig::cli_options_raw()` for adapters that own extras
(`mock-cli` reads its `script` fixture path this way).

## Per-adapter projection

`known_option_keys` no longer exists; the projection happens inside each
adapter's `build_command_string`.

| Adapter    | `--model` | `permission` projection                                                       | `additional_args`             | Adapter-specific extras |
|------------|-----------|-------------------------------------------------------------------------------|-------------------------------|--------------------------|
| `claude`   | `--model <m>` | `--permission-mode plan\|acceptEdits\|bypassPermissions\|default`         | appended verbatim             | —                        |
| `opencode` | `--model <m>` | no native flag — logged as a `tracing::warn` (set permission via `cli.command` if you need it) | appended verbatim             | —                        |
| `pi`       | `--model <m>` (e.g. `sonnet:high`) | no native flag — logged as a `tracing::warn`                                  | appended verbatim             | —                        |
| `mock`     | ignored   | ignored                                                                       | ignored                       | `script` (path to YAML/JSON event fixture; resolved against the workspace) |

`additional_args` ordering is: adapter's canonical argv first (model,
permission, session resume), then `additional_args` appended last. Each
token is `shell_escape`d individually before joining.

## Per-phase override: `phases[].cli.options`

The `phases[]` block in `WORKFLOW.md` front matter accepts a `cli.options`
sub-map that **mirrors the global `cli.options` schema exactly** (same
typed triplet, same legacy-key rejections). At dispatch time the
orchestrator looks up the phase for the current tracker state and
shallow-merges the override over the global map:

- Each key set in `phase.cli.options` REPLACES the corresponding global
  value (`model`, `permission`, `additional_args`).
- A key absent from the phase override keeps the global value.
- `additional_args` is treated atomically: a non-empty phase override
  replaces the entire global array (no concatenation).

The legacy `phases[].cli_options` (flat map at phase level) is hard-rejected
at parse time with an error pointing at the rename.

Implementation: `src/orchestrator/tick.rs` builds `phase_options` from the
active phase and threads it through `AgentRunner::run_turn`, which calls
`CliConfig::with_effective_options` to produce a per-turn config the adapter
sees in `build_command_string`.

## Adapter trait surface

Trait: `src/agent/cli/mod.rs::CliAdapter`. Every adapter overrides only the
hooks that diverge from the OpenCode reference shape:

| Method                  | claude | opencode | pi | mock |
|-------------------------|--------|----------|----|------|
| `kind()`                | own    | own      | own| own  |
| `binary_names()`        | own    | own      | own| own  |
| `validate()`            | default (leading-binary check via shared `validate_command_binary`) | default | default | default |
| `start_session()`       | default (synthetic `<kind>-<pid>-<ts>` id) | default | default | default |
| `run_turn()`            | default (delegates to `AgentBackend::run_turn`) | default | default | default |
| `stop_session()`        | default (no-op) | default | default | default |
| `build_command_string()`| own (`--print`, `--output-format stream-json`, `--add-dir`, `--model`, `--permission-mode`, `--resume` on UUID) | own (`opencode run`, `--format json`, `--dir`, `--model`, `--session` on UUID) | own (`pi --mode json`, `--model`, `--session` on UUID, no `--dir`) | default |
| `parse_stdout_line()`   | own (Claude `stream-json` envelope) | default (opencode-shaped) | own (pi JSONL: `session`, `message_update` text deltas, `turn_end`/`agent_end`) | — |
| `sanitize_prompt()`     | default (identity) | own (wraps `^--<flag>$` lines in backticks) | default | default |

Shared helpers live in `src/agent/cli/mod.rs`:

- `is_uuid` — single 8-4-4-4-12 hex check, used by claude / opencode / pi
  before splicing a session id into `--resume` / `--session`.
- `validate_command_binary` — single leading-binary identity check used as
  the trait's default `validate`.
- `append_flag(cmd, "--flag", value)` — shell-escapes the value.
- `append_additional_args(cmd, args)` — shell-escapes each token.

## Reference invocations

```yaml
cli:
  command: claude
  options:
    model: claude-haiku-4.5
    permission: acceptEdits
    additional_args: ["--add-dir", "/extra"]
```

```yaml
cli:
  command: opencode run
  options:
    model: openrouter/anthropic/claude-haiku-4.5
    additional_args: ["--print"]
```

```yaml
cli:
  command: pi
  options:
    model: sonnet:high
    additional_args: ["--thinking", "high"]
```

```yaml
cli:
  command: mock-cli
  options:
    script: fixtures/run.yaml

phases:
  - name: build
    state: In Progress
    prompt: "Implement the LLD."
    cli:
      options:
        model: claude-opus-4-7
        permission: bypassPermissions
```
