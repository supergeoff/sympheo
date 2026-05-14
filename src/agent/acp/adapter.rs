/// §6.1 — Declarative trait every ACP adapter must implement.
///
/// An adapter encapsulates everything about a specific ACP agent binary:
/// how to spawn it, what capabilities to advertise, how to initialise a
/// session, and how to express static permission intent.
///
/// Adapters do NOT own a backend or manage connections — that is the
/// responsibility of [`super::connection::AcpConnection`].
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use agent_client_protocol::schema::{ClientCapabilities, Implementation, NewSessionRequest};

use crate::agent::cli::{CliOptions, Permission};
use crate::error::SympheoError;

/// Static hint returned by `AcpAdapter::static_permission_hint`.
///
/// Tells the permission handler whether this adapter wants a blanket
/// allow or reject before inspecting individual tool kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionHint {
    /// Let the per-tool matrix decide.
    UseMatrix,
    /// Skip the matrix and allow everything.
    AllowAll,
    /// Skip the matrix and reject everything.
    RejectAll,
}

/// §6.1 — Declarative adapter trait.
///
/// All methods are pure (no I/O except `preflight_check`). Implementors
/// describe the agent; `AcpConnection` drives the actual lifecycle.
pub trait AcpAdapter: Send + Sync {
    /// Short identifier used in logs and connection IDs (e.g. `"opencode"`, `"claude-agent-acp"`).
    fn kind(&self) -> &str;

    /// Build the [`Command`] used to spawn the agent process.
    ///
    /// **Spawn convention** (§6.1):
    /// - program = `"mise"`, args = `["exec", "--", <program>, <agent-args>...]`
    /// - `cli_env` is merged via `Command::env`
    /// - `cwd` is set by the caller after this method returns
    /// - No PATH override, no `mise which`, no absolute binary path
    fn spawn_spec(&self, cli_env: &HashMap<String, String>) -> Command;

    /// Capabilities this client advertises to the agent during `initialize`.
    ///
    /// Lot 1 defaults: `fs.{read_text_file, write_text_file} = false`, `terminal = false`.
    fn client_capabilities(&self) -> ClientCapabilities;

    /// Build the `session/new` request body for a given working directory.
    fn session_new_params(&self, cwd: PathBuf) -> NewSessionRequest;

    /// Optionally configure the model on an active connection (e.g. via
    /// `session/set_config_option`). No-op by default; override for adapters
    /// that support model selection at session time.
    fn apply_model(
        &self,
        _connection: &super::connection::AcpConnection,
        _options: &CliOptions,
    ) -> Result<(), SympheoError> {
        Ok(())
    }

    /// Static permission hint that lets the adapter short-circuit the per-tool
    /// matrix for modes where the answer is always the same (e.g. `BypassPermissions`
    /// → `AllowAll`).
    fn static_permission_hint(&self, permission: Option<Permission>) -> PermissionHint {
        match permission {
            Some(Permission::BypassPermissions) => PermissionHint::AllowAll,
            _ => PermissionHint::UseMatrix,
        }
    }

    /// Optional pre-flight check run once before connecting (e.g. verify the
    /// binary exists on PATH). Returns `Ok(())` by default.
    fn preflight_check(&self) -> Result<(), SympheoError> {
        Ok(())
    }
}

/// Construct the default Lot-1 [`ClientCapabilities`]:
/// `fs.{read_text_file, write_text_file} = false`, `terminal = false`.
///
/// All fields default to false/disabled, matching the Lot-1 constraint that
/// the ACP adapter uses its own internal tools rather than client-provided fs/terminal.
pub fn default_client_capabilities() -> ClientCapabilities {
    // ClientCapabilities::default() already sets fs = all-false, terminal = false.
    ClientCapabilities::default()
}

/// Construct the `clientInfo` block sent to `initialize`:
/// `{ name: "sympheo", version: <CARGO_PKG_VERSION> }`.
pub fn sympheo_client_info() -> Implementation {
    Implementation::new("sympheo", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct MinimalAdapter;

    impl AcpAdapter for MinimalAdapter {
        fn kind(&self) -> &str {
            "test-minimal"
        }

        fn spawn_spec(&self, cli_env: &HashMap<String, String>) -> Command {
            let mut cmd = Command::new("mise");
            cmd.args(["exec", "--", "test-agent"]);
            for (k, v) in cli_env {
                cmd.env(k, v);
            }
            cmd
        }

        fn client_capabilities(&self) -> ClientCapabilities {
            default_client_capabilities()
        }

        fn session_new_params(&self, cwd: PathBuf) -> NewSessionRequest {
            NewSessionRequest::new(cwd)
        }
    }

    #[test]
    fn adapter_kind() {
        assert_eq!(MinimalAdapter.kind(), "test-minimal");
    }

    #[test]
    fn spawn_spec_uses_mise_exec() {
        let cmd = MinimalAdapter.spawn_spec(&HashMap::new());
        let prog = cmd.get_program().to_string_lossy().into_owned();
        assert_eq!(prog, "mise");
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args[0], "exec");
        assert_eq!(args[1], "--");
        assert_eq!(args[2], "test-agent");
    }

    #[test]
    fn spawn_spec_merges_env() {
        let env = HashMap::from([("FOO".to_string(), "bar".to_string())]);
        let cmd = MinimalAdapter.spawn_spec(&env);
        let found = cmd.get_envs().any(|(k, v)| k == "FOO" && v == Some("bar".as_ref()));
        assert!(found);
    }

    #[test]
    fn client_capabilities_fs_disabled() {
        let caps = MinimalAdapter.client_capabilities();
        assert!(!caps.fs.read_text_file);
        assert!(!caps.fs.write_text_file);
        assert!(!caps.terminal);
    }

    #[test]
    fn session_new_params_sets_cwd() {
        let cwd = PathBuf::from("/tmp/workspace");
        let params = MinimalAdapter.session_new_params(cwd.clone());
        assert_eq!(params.cwd, cwd);
    }

    #[test]
    fn static_permission_hint_bypass() {
        let hint = MinimalAdapter.static_permission_hint(Some(Permission::BypassPermissions));
        assert_eq!(hint, PermissionHint::AllowAll);
    }

    #[test]
    fn static_permission_hint_other_modes_use_matrix() {
        for mode in [
            Some(Permission::AcceptEdits),
            Some(Permission::Plan),
            Some(Permission::Default),
            None,
        ] {
            let hint = MinimalAdapter.static_permission_hint(mode);
            assert_eq!(hint, PermissionHint::UseMatrix, "mode={mode:?}");
        }
    }

    #[test]
    fn preflight_check_default_ok() {
        assert!(MinimalAdapter.preflight_check().is_ok());
    }

    #[test]
    fn sympheo_client_info_fields() {
        let info = sympheo_client_info();
        assert_eq!(info.name, "sympheo");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn default_client_capabilities_matches_spec() {
        let caps = default_client_capabilities();
        assert!(!caps.fs.read_text_file);
        assert!(!caps.fs.write_text_file);
        assert!(!caps.terminal);
    }
}
