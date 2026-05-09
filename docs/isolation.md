# Sympheo — Local execution isolation model

This document describes the per-worker filesystem and environment isolation
applied by the **local execution backend**. SPEC §15.1 requires implementations
to "document the selected behavior" for trust boundary and approval posture;
this is that documentation.

It does NOT describe the Daytona backend (which provides isolation through a
remote sandbox API at the infrastructure level). It is also unrelated to the
SPEC §11.5 trust boundary around tracker writes (Sympheo does not validate
agent-claimed outcomes — that posture is unchanged).

## Threat model

The CLI subprocess (`opencode`, future `pi`, etc.) runs **arbitrary
LLM-driven shell commands** in the workspace. It is NOT trusted to leave
the workspace alone. Specifically, we want to prevent:

1. **Cross-worker contamination** — worker A reading or mutating worker B's
   workspace, agent state, or session cache.
2. **Cross-worker / host config contamination** — the agent reading the host
   operator's `~/.config/opencode`, `~/.local/share/opencode`, or any other
   user-scoped config / cache / state directory.
3. **Credential leaks** — host environment variables that look like API keys
   (`ANTHROPIC_API_KEY`, `AWS_ACCESS_KEY_ID`, `GITHUB_TOKEN`, etc.) being
   inherited by the subprocess unless explicitly listed in
   `cli.env` (§5.3.6).
4. **Arbitrary PATH discovery** — the agent invoking host-local binaries the
   operator did not intend to expose.

It does NOT prevent:

- Filesystem escape via absolute path access (e.g. `cat /etc/passwd`). For
  that you need OS-level sandboxing (bwrap, chroot, container, Daytona).
- Network access. The subprocess inherits the kernel's network stack and
  can reach anything its TCP/IP stack can reach. For that you need a
  network namespace or a firewall.

For stronger guarantees, use the Daytona backend or run sympheo inside a
container with the appropriate restrictions.

## Implementation summary

For every CLI turn, `LocalBackend::run_turn` does the following before
spawning the subprocess (`src/agent/backend/local.rs`,
`src/workspace/isolation.rs`):

1. **Provision a per-workspace HOME subtree** under
   `<workspace>/.sympheo-home/` with these subdirectories:
   - `.sympheo-home/.config` → `XDG_CONFIG_HOME`
   - `.sympheo-home/.local/share` → `XDG_DATA_HOME`
   - `.sympheo-home/.cache` → `XDG_CACHE_HOME`
   - `.sympheo-home/.local/state` → `XDG_STATE_HOME`
   - `.sympheo-home/.local/bin` → first entry in default `PATH`

   Idempotent: created on first turn, reused on subsequent turns.

2. **Clear the inherited environment** with `Command::env_clear()`.

3. **Re-populate** with the layered map produced by
   `workspace::isolation::build_isolated_env`:

   | Layer | Source | Notes |
   |---|---|---|
   | 1 | Host passthrough whitelist | `LANG`, `LANGUAGE`, `LC_*`, `TERM`, `TZ`, `USER`, `LOGNAME` |
   | 2 | Sympheo-managed | `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME` (all under workspace) |
   | 3 | Default `PATH` | `<HOME>/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin` |
   | 4 | `cli.env` (§5.3.6) | Operator overrides — always wins |

4. **Spawn `bash -lc <cli.command>`** in the workspace directory. The
   `-l` (login) flag still loads `/etc/profile`, but `~/.bash_profile` and
   `~/.profile` are loaded from the **scoped HOME** (which is empty by
   default), not the operator's home.

## Operator overrides

Anything in `cli.env` in `WORKFLOW.md` wins over Sympheo's defaults. To
expose `GITHUB_TOKEN` to the agent so it can call `gh api`:

```yaml
cli:
  command: opencode run
  env:
    GITHUB_TOKEN: $SYMPHEO_GITHUB_TOKEN
```

To extend `PATH` (e.g. add `~/.cargo/bin`):

```yaml
cli:
  command: opencode run
  env:
    PATH: "/home/runner/.cargo/bin:/usr/local/bin:/usr/bin:/bin"
```

`$VAR` indirection in values is resolved per §6.1.

## Tests

- `src/workspace/isolation.rs::tests` — unit tests for the env-build helper
  (HOME/XDG scoping, default PATH, override precedence, no credential leak).
- `src/agent/backend/local.rs::tests::test_local_backend_env_isolation` —
  integration test that launches a fake CLI under `LocalBackend::run_turn`
  with a credential-shaped host env var set, asserts HOME is scoped to the
  workspace and the credential did NOT leak.
- `src/agent/backend/local.rs::tests::test_local_backend_cli_env_overrides_pass_through`
  — asserts an explicit `cli.env.GITHUB_TOKEN` override reaches the
  subprocess.

## Migration impact

The first turn after upgrading creates the `.sympheo-home` subtree under
each existing workspace. Old `~/.config/opencode/...` data on the host is
not migrated — the agent starts fresh inside the workspace. If the
workflow previously relied on a host-side opencode config file, that file
must be either:

- Re-created inside `<workspace>/.sympheo-home/.config/opencode/` (e.g.
  via the `after_create` hook), or
- Re-introduced as `cli.env` overrides if it was env-driven configuration.
