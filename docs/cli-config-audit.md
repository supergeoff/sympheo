# CLI config audit — what's actually wired vs what's only parsed

Date: 2026-05-11. Scope: every `cli.*` field that `ServiceConfig` parses
from `WORKFLOW.md`, plus the per-phase `phases[].cli_options` override,
mapped against what each `CliAdapter` (`claude`, `opencode`, `mock`,
`pi`) and the `LocalBackend` actually consume at runtime.

The trigger for this audit was the question: *"if I drop
`cli.options.model: sonnet` in my workflow, does it pin the model?"*
Answer: no. The reason is below.

## TL;DR

- The only knob that actually changes the agent argv at runtime is
  `cli.command`. Everything you put inside `cli.options` for `claude`
  or `opencode` is silently dropped on the floor — same for
  `cli.args`, and same for `phases[].cli_options`.
- `cli.env` works, but only via `LocalBackend` (subprocess env merge).
  `MockBackend` ignores it.
- The three `cli.*_timeout_ms` keys work; they're enforced by
  `LocalBackend`.
- `mock` is the only adapter that actually reads `cli.options`
  (specifically `script`). `claude` / `opencode` declare option keys
  in `known_option_keys()` (so they don't warn) but never thread them
  into the argv.

## Field-by-field table (top-level `cli.*`)

Source of truth: `src/config/typed.rs`, accessors prefixed `cli_*`.

| Field                   | Parser accessor              | Default      | Real consumer                                                                     | Status |
|-------------------------|------------------------------|--------------|------------------------------------------------------------------------------------|--------|
| `cli.command`           | `cli_command()`              | (required)   | adapter `validate()` + `build_command_string()`; spawned by `LocalBackend` via `bash -lc` | ✅ live |
| `cli.args`              | `cli_args()`                 | `[]`         | loaded into `CliConfig.args` and **never read again**                              | 💀 dead |
| `cli.env`               | `cli_env()`                  | `{}`         | `LocalBackend` merges into subprocess env. `MockBackend` ignores.                  | ⚠️ partial (LocalBackend only) |
| `cli.options`           | `cli_options()`              | `{}`         | forwarded to adapter; see per-adapter table below                                  | ⚠️ adapter-dependent |
| `cli.turn_timeout_ms`   | `cli_turn_timeout_ms()`      | `3600000`    | `LocalBackend` turn-wide timeout                                                   | ✅ live |
| `cli.read_timeout_ms`   | `cli_read_timeout_ms()`      | `5000`       | `LocalBackend` per-stdout-line timeout                                             | ✅ live |
| `cli.stall_timeout_ms`  | `cli_stall_timeout_ms()`     | `300000`     | `LocalBackend` stall detection                                                     | ✅ live |

## `cli.options.*` per adapter — declared vs effective

`known_option_keys()` only suppresses the "unknown option" warning
(see `warn_unknown_options()` in `src/agent/cli/mod.rs`). Declaring a
key there does **not** mean the adapter actually does anything with
it.

| Adapter    | `known_option_keys` (declared)                  | Actually consumed | Gap                |
|------------|-------------------------------------------------|-------------------|--------------------|
| `claude`   | `model`, `permission_mode`, `additional_args`   | (none)            | 💀 3 dead keys     |
| `opencode` | `model`, `permissions`, `mcp_servers`           | (none)            | 💀 3 dead keys     |
| `mock`     | `script`                                        | `script` (read by `MockBackend`) | ✅ wired |
| `pi`       | (default `[]`)                                  | —                 | n/a                |

The most user-hostile shape here is that the dead keys are *declared*
as known, so the operator gets no warning when they set
`cli.options.model: sonnet` in a real workflow. The value is parsed,
forwarded, and then silently discarded.

**Workaround for today**: pin the model inline in `cli.command`:

```yaml
cli:
  command: claude --model sonnet
# or
  command: opencode run --model openrouter/anthropic/claude-haiku-4.5
```

(The e2e `Generate Workflow Md For Opencode Code Phase` generator
does exactly this, with a comment pointing back at the gap.)

## `phases[].cli_options` — parsed, documented, never read

`src/workflow/phase.rs:7`:

> *"Each phase maps a tracker state to a prompt fragment (interpolated
> as `{{ phase.prompt }}` into the global template), post-turn
> verifications, and per-phase cli_options overrides."*

`src/workflow/phase.rs:14`:

```rust
pub cli_options: serde_json::Map<String, serde_json::Value>,
```

`src/workflow/phase.rs:46` — populated from the YAML front matter via
`resolver::get_string_map(m, "cli_options")`.

Then: `rg "phase\.cli_options|p\.cli_options" src/` filtered through
non-test, non-comment, returns zero hits. The only references outside
`phase.rs`'s own tests are:

- `src/orchestrator/tick.rs:1358` — inside a `#[cfg(test)]` block,
  building a `Phase` literal with an empty map.

So: the field is parsed, the documentation calls it a "per-phase
override", and no production code path reads it. Even `MockBackend`,
which is the one place that reads top-level `cli.options`, does not
look at the phase-level equivalent.

Implication: any per-phase `cli_options:` block in a `WORKFLOW.md`
today is no-op. Worse, since the parser accepts it silently, an
operator reading the PRD will reasonably believe they can scope a
permission or a model to a single phase.

## Adapter trait surface — implemented vs default

Trait: `src/agent/cli/mod.rs:101`.

| Method                  | claude                    | opencode                  | mock              | pi                |
|-------------------------|---------------------------|---------------------------|-------------------|-------------------|
| `kind()`                | `"claude"`                | `"opencode"`              | `"mock-cli"`      | `"pi"`            |
| `binary_names()`        | `["claude"]`              | `["opencode"]`            | `["mock-cli"]`    | `["pi"]`          |
| `validate()`            | own impl                  | own impl                  | own impl          | own impl (checks `"pi run"`) |
| `known_option_keys()`   | 3 declarative-only        | 3 declarative-only        | active (`script`) | default `[]`      |
| `start_session()`       | trait default (`<kind>-<pid>-<ts>` synthetic id) | trait default | mock-specific | trait default     |
| `run_turn()`            | trait default (`LocalBackend`) | trait default        | mock-specific     | trait default     |
| `stop_session()`        | trait default (no-op)     | trait default             | mock-specific     | trait default     |
| `build_command_string()`| own impl, UUID guard on `--resume` | own impl, UUID guard on `--session` | — | trait default |
| `parse_stdout_line()`   | own impl (`stream-json`)  | own impl (json events)    | —                 | trait default     |
| `sanitize_prompt()`     | identity (default)        | `sanitize_prompt_for_opencode` | identity     | identity          |

The session-id synthetic-handle pattern is duplicated across `claude`
and `opencode`, and both adapters have a near-identical
`is_uuid()`-shape guard to skip `--resume` / `--session` when the
synthetic handle isn't a real UUID. That helper is duplicated, not
shared — a tiny consolidation opportunity but not the bordel.

## Why the bordel feels like a bordel

Three intersecting causes:

1. **Two parallel "options" channels** (`cli.options`,
   `phases[].cli_options`), only one of them ever consumed, and only
   for one adapter (`mock`). The other adapters advertise option
   *keys* but don't implement them, which reads like support.
2. **Silent acceptance + no warning** for keys that are declared but
   not threaded. `known_option_keys()` should mean "the adapter does
   something with this key"; in practice it means "the adapter
   doesn't complain about this key". Those are different contracts.
3. **`cli.args`** is fully parsed and stored in `CliConfig.args` but
   no code reads `CliConfig.args` after that. It looks supported,
   isn't.

The PRD's intent for `phases[].cli_options` was probably: merge
`phases[<active>].cli_options` over the top-level `cli.options` at
dispatch time, hand the merged map to the adapter. That merge layer
doesn't exist anywhere.

## Recommended remediation (not yet implemented)

Ordered cheapest → most invasive.

1. **Strip the declarative-only entries from `known_option_keys()`**
   for `claude` and `opencode`. With the keys gone, setting
   `cli.options.model` in a workflow will at least produce the
   "unknown option" warning, which is honest. Cost: 2 lines + a
   couple of tests.

2. **Drop `cli.args` from the spec** OR wire it into
   `CliAdapter::build_command_string` as a trailing append. Pick one,
   document the choice. Cost: small.

3. **Drop `phases[].cli_options` from the spec** OR implement the
   per-phase override merge. The merge is straightforward —
   `tick.rs` already looks up `phase_for_state` to inject
   `SYMPHEO_PHASE_NAME` into hook env, so it has the right entry
   point. Cost: one merge function + adapter plumbing to thread the
   merged map. Probably the biggest win because it's the most
   user-facing surprise.

4. **Implement the declarative-only keys for `claude` and `opencode`**:
   `model`, `permission_mode`, `additional_args`, `permissions`,
   `mcp_servers`. Each adapter's `build_command_string` would
   translate keys it owns into argv flags. Cost: moderate, but pays
   off as soon as anyone wants a per-phase model.

5. **Consolidate the duplicated `is_uuid` guard** between `claude.rs`
   and `opencode.rs` into a single helper in `cli/mod.rs`. Cosmetic.

Items 1, 2 and 3 together would already remove the "feels like
bordel" surface, even without item 4. Item 4 is what lets the
docstring of `Phase::cli_options` actually mean what it says.

## Reproduction commands

```bash
# Show the dead field
grep -rn 'phase\.cli_options\|p\.cli_options' src/ \
  | grep -v test | grep -v '//'

# Show the dead args field
grep -rn 'CliConfig\.args\|config\.args' src/ \
  | grep -v test

# Show what each adapter declares vs implements
grep -nE 'known_option_keys|build_command_string|\.options' \
  src/agent/cli/claude.rs src/agent/cli/opencode.rs

# Confirm only mock actually reads cli.options
grep -rn 'cli_options()' src/
```
