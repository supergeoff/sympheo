/// §6.3 — ACP connection wrapper.
///
/// `AcpConnection` wraps an active [`agent_client_protocol::Client`] connection
/// to an ACP agent process.  It owns the lifecycle:
/// - `initialize` handshake (version check)
/// - `session/new`
/// - `session/prompt` / `session/update` dispatch
/// - `session/cancel` and `session/close` (if announced by agent capabilities)
///
/// **Lot 3 scope**: constructor + version check.  Session management methods
/// are stubs that will be wired up in later lots.
use std::collections::HashMap;
use std::path::PathBuf;

use agent_client_protocol::schema::{InitializeRequest, ProtocolVersion};
use agent_client_protocol::{Agent, Client, ConnectionTo};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::debug;

use crate::agent::acp::adapter::{AcpAdapter, sympheo_client_info};
use crate::error::SympheoError;

/// Minimum ACP protocol version Sympheo accepts.
/// V0 is rejected; V1 and above are accepted.
pub const MIN_PROTOCOL_VERSION: u16 = 1;

// ---------------------------------------------------------------------------
// Version check (pure — no I/O, fully unit-testable)
// ---------------------------------------------------------------------------

/// Return `Ok(())` if `version` meets the minimum, or
/// `Err(SympheoError::AcpProtocolVersionUnsupported)` otherwise.
///
/// V0 is rejected.  V1, V2, and any future version ≥ V1 are accepted.
pub fn check_protocol_version(version: ProtocolVersion) -> Result<(), SympheoError> {
    if version < ProtocolVersion::V1 {
        return Err(SympheoError::AcpProtocolVersionUnsupported(format!(
            "V{version} (minimum required: V{MIN_PROTOCOL_VERSION})"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Connection struct
// ---------------------------------------------------------------------------

/// Handle to an active ACP connection.
///
/// Constructed via [`AcpConnection::connect`].  In Lot 3 this is a thin
/// shell; session-level operations (`run_turn`, `cancel`, `close`) will be
/// added in subsequent lots once the backend infrastructure (Lot 4) is in
/// place.
pub struct AcpConnection {
    // Reserved for connection state added in later lots.
    // Kept private so the public API can be extended without breaking callers.
    _private: (),
}

impl AcpConnection {
    /// Spawn the agent process described by `adapter`, connect over stdio,
    /// perform the ACP `initialize` handshake, and return a live connection.
    ///
    /// Fails with [`SympheoError::AcpProtocolVersionUnsupported`] if the
    /// agent replies with a protocol version below `MIN_PROTOCOL_VERSION`.
    pub async fn connect(
        adapter: &dyn AcpAdapter,
        cli_env: &HashMap<String, String>,
        _cwd: PathBuf,
    ) -> Result<Self, SympheoError> {
        let mut cmd = adapter.spawn_spec(cli_env);
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit());

        let mut child = tokio::process::Command::from(cmd)
            .spawn()
            .map_err(|e| SympheoError::TurnLaunchFailed(e.to_string()))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SympheoError::TurnLaunchFailed("child stdin not available".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SympheoError::TurnLaunchFailed("child stdout not available".into()))?;

        let transport =
            agent_client_protocol::ByteStreams::new(stdin.compat_write(), stdout.compat());

        let client_capabilities = adapter.client_capabilities();
        let client_info = sympheo_client_info();

        Client
            .connect_with(transport, |cx: ConnectionTo<Agent>| async move {
                let init_req = InitializeRequest::new(ProtocolVersion::V1)
                    .client_capabilities(client_capabilities)
                    .client_info(client_info);

                let resp = cx.send_request(init_req).block_task().await.map_err(|e| {
                    agent_client_protocol::util::internal_error(format!("initialize failed: {e}"))
                })?;

                debug!(
                    protocol_version = ?resp.protocol_version,
                    "ACP initialize OK"
                );

                check_protocol_version(resp.protocol_version)
                    .map_err(|e| agent_client_protocol::util::internal_error(e.to_string()))?;

                Ok(AcpConnection { _private: () })
            })
            .await
            .map_err(|e| SympheoError::SessionStartFailed(e.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Version check is pure — no async needed.

    #[test]
    fn v0_is_rejected() {
        let err = check_protocol_version(ProtocolVersion::V0).unwrap_err();
        match err {
            SympheoError::AcpProtocolVersionUnsupported(msg) => {
                assert!(
                    msg.contains("V0"),
                    "message should name the offending version: {msg}"
                );
            }
            other => panic!("expected AcpProtocolVersionUnsupported, got {other:?}"),
        }
    }

    #[test]
    fn v1_is_accepted() {
        assert!(check_protocol_version(ProtocolVersion::V1).is_ok());
    }

    #[test]
    fn v2_is_accepted() {
        // V2 is not a named constant yet, but any version ≥ V1 must be accepted.
        let v2 = ProtocolVersion::from(2u16);
        assert!(check_protocol_version(v2).is_ok());
    }

    #[test]
    fn min_protocol_version_constant_is_one() {
        assert_eq!(MIN_PROTOCOL_VERSION, 1);
    }
}
