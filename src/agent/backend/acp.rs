/// §6.5 — ACP production backend: long-lived child process + bounded MPSC + timeouts.
///
/// One child process and one ACP JSON-RPC connection are maintained per worker-run.
/// Turns are dispatched via a bounded MPSC channel (capacity 1024) to a reactor task
/// that drives the ACP session loop.  Cooperative cancel uses `session/cancel` with a
/// grace period before `killpg`.
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::{
    CancelNotification, InitializeRequest, ProtocolVersion, RequestPermissionRequest,
    SessionNotification, StopReason,
};
use agent_client_protocol::util::MatchDispatch;
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo};
use async_trait::async_trait;
use tokio::sync::mpsc::Sender;
use tokio::sync::{mpsc, oneshot};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{debug, warn};

use crate::agent::acp::adapter::{AcpAdapter, sympheo_client_info};
use crate::agent::acp::connection::check_protocol_version;
use crate::agent::acp::permission::handle_request_permission;
use crate::agent::acp::translator::translate;
use crate::agent::backend::AgentBackend;
use crate::agent::cli::CliOptions;
use crate::agent::parser::{AgentEvent, EmittedEvent, TurnOutcome, TurnResult};
use crate::error::SympheoError;
use crate::tracker::model::Issue;

/// Bounded capacity of the prompt command channel.
pub const PROMPT_CHANNEL_CAP: usize = 1024;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AcpBackendConfig {
    pub session_start_timeout: Duration,
    pub read_timeout: Duration,
    pub tool_progress_timeout: Duration,
    pub cancel_grace: Duration,
    pub turn_timeout: Duration,
}

impl Default for AcpBackendConfig {
    fn default() -> Self {
        Self {
            session_start_timeout: Duration::from_secs(30),
            read_timeout: Duration::from_secs(30),
            tool_progress_timeout: Duration::from_secs(60),
            cancel_grace: Duration::from_secs(5),
            turn_timeout: Duration::from_secs(1800),
        }
    }
}

impl AcpBackendConfig {
    pub fn from_service_config(cfg: &crate::config::typed::ServiceConfig) -> Self {
        Self {
            session_start_timeout: Duration::from_millis(cfg.cli_session_start_timeout_ms()),
            read_timeout: Duration::from_millis(cfg.cli_read_timeout_ms()),
            tool_progress_timeout: Duration::from_millis(cfg.cli_tool_progress_timeout_ms()),
            cancel_grace: Duration::from_millis(cfg.cli_cancel_grace_ms()),
            turn_timeout: Duration::from_millis(cfg.cli_turn_timeout_ms()),
        }
    }
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

struct PromptCmd {
    prompt: String,
    cli_options: CliOptions,
    event_tx: Sender<EmittedEvent>,
    cancelled: Arc<AtomicBool>,
    result_tx: oneshot::Sender<Result<(TurnOutcome, Option<String>), SympheoError>>,
}

struct AcpSessionState {
    cmd_tx: mpsc::Sender<PromptCmd>,
    child_pgid: u32,
    acp_session_id: String,
    _thread: std::thread::JoinHandle<()>,
}

// ---------------------------------------------------------------------------
// AcpBackend
// ---------------------------------------------------------------------------

pub struct AcpBackend {
    adapter: Arc<dyn AcpAdapter>,
    config: AcpBackendConfig,
    cli_env: HashMap<String, String>,
    state: Mutex<Option<AcpSessionState>>,
}

impl AcpBackend {
    pub fn new(
        adapter: Arc<dyn AcpAdapter>,
        config: AcpBackendConfig,
        cli_env: HashMap<String, String>,
    ) -> Self {
        Self {
            adapter,
            config,
            cli_env,
            state: Mutex::new(None),
        }
    }

    /// Spawn child process and open ACP session; must be called once before `run_turn`.
    pub async fn start_session(&self, cwd: PathBuf) -> Result<(), SympheoError> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<PromptCmd>(PROMPT_CHANNEL_CAP);
        let (ready_tx, ready_rx) = oneshot::channel::<Result<(u32, String), SympheoError>>();

        let adapter = Arc::clone(&self.adapter);
        let config = self.config.clone();
        let cli_env = self.cli_env.clone();

        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("acp reactor runtime");
            let local = tokio::task::LocalSet::new();
            local.block_on(
                &rt,
                run_reactor(adapter, config, cli_env, cwd, cmd_rx, ready_tx),
            );
        });

        let ready_result = tokio::time::timeout(self.config.session_start_timeout, ready_rx)
            .await
            .map_err(|_| SympheoError::SessionStartFailed("session start timed out".into()))?
            .map_err(|_| SympheoError::SessionStartFailed("reactor exited before ready".into()))?;

        let (child_pgid, acp_session_id) = ready_result?;

        let mut state = self.state.lock().unwrap();
        *state = Some(AcpSessionState {
            cmd_tx,
            child_pgid,
            acp_session_id,
            _thread: thread,
        });

        Ok(())
    }

    /// Kill the child process group and drop session state.
    pub fn stop_session(&self) {
        let state = self.state.lock().unwrap().take();
        if let Some(s) = state {
            kill_pgid(s.child_pgid);
        }
    }
}

#[async_trait]
impl AgentBackend for AcpBackend {
    fn kind(&self) -> &'static str {
        "acp"
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_turn(
        &self,
        _issue: &Issue,
        prompt: &str,
        _session_id: Option<&str>,
        _workspace_path: &Path,
        cancelled: Arc<AtomicBool>,
        event_tx: Sender<EmittedEvent>,
        cli_options: &CliOptions,
    ) -> Result<TurnResult, SympheoError> {
        let (result_tx, result_rx) =
            oneshot::channel::<Result<(TurnOutcome, Option<String>), SympheoError>>();

        let cmd = PromptCmd {
            prompt: prompt.to_string(),
            cli_options: cli_options.clone(),
            event_tx,
            cancelled,
            result_tx,
        };

        let (cmd_tx, child_pgid, acp_session_id) = {
            let state = self.state.lock().unwrap();
            let s = state
                .as_ref()
                .ok_or_else(|| SympheoError::SessionStartFailed("no active session".into()))?;
            (s.cmd_tx.clone(), s.child_pgid, s.acp_session_id.clone())
        };

        cmd_tx
            .send(cmd)
            .await
            .map_err(|_| SympheoError::TurnFailed("reactor channel closed".into()))?;

        let turn_result = tokio::time::timeout(self.config.turn_timeout, result_rx)
            .await
            .map_err(|_| {
                kill_pgid(child_pgid);
                SympheoError::TurnTotalTimeout
            })?
            .map_err(|_| SympheoError::TurnFailed("reactor result channel closed".into()))?;

        let (outcome, last_message) = turn_result?;

        Ok(TurnResult {
            session_id: acp_session_id,
            turn_id: String::new(),
            outcome,
            last_message,
            usage: None,
            error: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Reactor
// ---------------------------------------------------------------------------

async fn run_reactor(
    adapter: Arc<dyn AcpAdapter>,
    config: AcpBackendConfig,
    cli_env: HashMap<String, String>,
    cwd: PathBuf,
    mut cmd_rx: mpsc::Receiver<PromptCmd>,
    ready_tx: oneshot::Sender<Result<(u32, String), SympheoError>>,
) {
    let mut spawn_cmd = adapter.spawn_spec(&cli_env);
    spawn_cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        spawn_cmd.process_group(0);
    }

    let mut child = match tokio::process::Command::from(spawn_cmd).spawn() {
        Ok(c) => c,
        Err(e) => {
            ready_tx
                .send(Err(SympheoError::TurnLaunchFailed(e.to_string())))
                .ok();
            return;
        }
    };

    let child_pid = child.id().unwrap_or(0);

    let stdin = match child.stdin.take() {
        Some(s) => s,
        None => {
            ready_tx
                .send(Err(SympheoError::TurnLaunchFailed("no stdin".into())))
                .ok();
            return;
        }
    };
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            ready_tx
                .send(Err(SympheoError::TurnLaunchFailed("no stdout".into())))
                .ok();
            return;
        }
    };

    let transport = ByteStreams::new(stdin.compat_write(), stdout.compat());
    let adapter_kind = adapter.kind().to_string();
    let client_capabilities = adapter.client_capabilities();
    let session_params = adapter.session_new_params(cwd);
    let read_timeout = config.read_timeout;

    let result = Client
        .connect_with(transport, async move |cx: ConnectionTo<Agent>| {
            // ACP initialize
            let init_req = InitializeRequest::new(ProtocolVersion::V1)
                .client_capabilities(client_capabilities)
                .client_info(sympheo_client_info());

            let init_resp =
                tokio::time::timeout(read_timeout, cx.send_request(init_req).block_task())
                    .await
                    .map_err(|_| {
                        agent_client_protocol::util::internal_error("initialize timed out")
                    })?
                    .map_err(|e| {
                        agent_client_protocol::util::internal_error(format!("initialize: {e}"))
                    })?;

            debug!(protocol_version = ?init_resp.protocol_version, "ACP initialize OK");

            check_protocol_version(init_resp.protocol_version)
                .map_err(|e| agent_client_protocol::util::internal_error(e.to_string()))?;

            // session/new
            let new_sess_resp = tokio::time::timeout(
                read_timeout,
                cx.send_request_to(Agent, session_params).block_task(),
            )
            .await
            .map_err(|_| agent_client_protocol::util::internal_error("session/new timed out"))?
            .map_err(|e| {
                agent_client_protocol::util::internal_error(format!("session/new: {e}"))
            })?;

            let acp_session_id = new_sess_resp.session_id.clone();
            let mut session = cx.attach_session(new_sess_resp, vec![])?;

            debug!(session_id = %acp_session_id, "ACP session ready");

            // Signal start_session caller
            ready_tx
                .send(Ok((child_pid, acp_session_id.to_string())))
                .ok();

            // Main turn loop — exits when cmd_rx is closed (stop_session / drop)
            while let Some(cmd) = cmd_rx.recv().await {
                process_one_turn(&mut session, cmd, &config, &adapter_kind, child_pid).await;
            }

            Ok(())
        })
        .await;

    if let Err(e) = result {
        warn!(error = %e, "ACP reactor exited with error");
    }
}

// ---------------------------------------------------------------------------
// Turn processor
// ---------------------------------------------------------------------------

async fn process_one_turn<Link>(
    session: &mut agent_client_protocol::ActiveSession<'static, Link>,
    cmd: PromptCmd,
    config: &AcpBackendConfig,
    adapter_kind: &str,
    child_pgid: u32,
) where
    Link: agent_client_protocol::role::HasPeer<Agent> + agent_client_protocol::role::HasPeer<Link>,
{
    let PromptCmd {
        prompt,
        cli_options,
        event_tx,
        cancelled,
        result_tx,
    } = cmd;

    let permission = cli_options.permission;

    if let Err(e) = session.send_prompt(&prompt) {
        let _ = result_tx.send(Err(SympheoError::TurnFailed(e.to_string())));
        return;
    }

    let acp_session_id = session.session_id().clone();
    let mut cancel_sent = false;
    let last_message: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    loop {
        // Send cooperative cancel exactly once
        if cancelled.load(Ordering::Relaxed) && !cancel_sent {
            if let Err(e) = session
                .connection()
                .send_notification(CancelNotification::new(acp_session_id.clone()))
            {
                warn!(error = %e, "failed to send ACP cancel notification");
            }
            cancel_sent = true;
        }

        let read_timeout = if cancel_sent {
            config.cancel_grace
        } else {
            config.tool_progress_timeout
        };

        match tokio::time::timeout(read_timeout, session.read_update()).await {
            Ok(Ok(agent_client_protocol::SessionMessage::StopReason(reason))) => {
                let outcome = map_stop_reason(reason, cancel_sent);
                let lm = last_message.borrow().clone();
                let _ = result_tx.send(Ok((outcome, lm)));
                return;
            }

            Ok(Ok(agent_client_protocol::SessionMessage::SessionMessage(dispatch))) => {
                let now = chrono::Utc::now().timestamp_millis();
                let sid = acp_session_id.to_string();
                let perm = permission;
                let kind = adapter_kind.to_string();
                let cancel = cancel_sent;
                let etx = event_tx.clone();
                let pid = Some(child_pgid);
                let last_msg = Rc::clone(&last_message);

                if let Err(e) = MatchDispatch::new(dispatch)
                    .if_notification(async move |notif: SessionNotification| {
                        let events = translate(&sid, notif.update, now);
                        for event in &events {
                            if let AgentEvent::Text { part, .. } = event {
                                *last_msg.borrow_mut() = Some(part.text.clone());
                            }
                        }
                        for event in events {
                            let _ = etx.send(EmittedEvent::with_pid(event, pid)).await;
                        }
                        Ok(())
                    })
                    .await
                    .if_request(async move |req: RequestPermissionRequest, responder| {
                        handle_request_permission(req, responder, perm, &kind, cancel)
                    })
                    .await
                    .otherwise_ignore()
                {
                    warn!(error = %e, "ACP dispatch error in turn");
                }
            }

            // Wildcard arm required by #[non_exhaustive] on SessionMessage
            Ok(Ok(_)) => {}

            Ok(Err(e)) => {
                let _ = result_tx.send(Err(SympheoError::TurnFailed(e.to_string())));
                return;
            }

            // Grace period elapsed after cooperative cancel — force kill
            Err(_) if cancel_sent => {
                kill_pgid(child_pgid);
                let _ = result_tx.send(Err(SympheoError::TurnCancelled));
                return;
            }

            // Tool-progress stall timeout
            Err(_) => {
                let _ = result_tx.send(Err(SympheoError::TurnReadTimeout));
                return;
            }
        }
    }
}

fn map_stop_reason(reason: StopReason, cancel_sent: bool) -> TurnOutcome {
    match reason {
        StopReason::Cancelled => TurnOutcome::Cancelled,
        StopReason::EndTurn if cancel_sent => TurnOutcome::Cancelled,
        StopReason::EndTurn => TurnOutcome::Succeeded,
        _ => TurnOutcome::Failed,
    }
}

// ---------------------------------------------------------------------------
// Process group kill
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn kill_pgid(pgid: u32) {
    if pgid == 0 {
        return;
    }
    // SAFETY: libc::kill with negative pid sends signal to the process group.
    // pgid is always a valid u32 obtained from child.id().
    unsafe {
        libc::kill(-(pgid as i32), libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_pgid(_pgid: u32) {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use super::*;
    use crate::agent::acp::adapter::{AcpAdapter, default_client_capabilities};
    use crate::agent::cli::Permission;

    struct StubAdapter;

    impl AcpAdapter for StubAdapter {
        fn kind(&self) -> &str {
            "stub"
        }

        fn spawn_spec(&self, _cli_env: &HashMap<String, String>) -> std::process::Command {
            std::process::Command::new("true")
        }

        fn client_capabilities(&self) -> agent_client_protocol::schema::ClientCapabilities {
            default_client_capabilities()
        }

        fn session_new_params(
            &self,
            cwd: std::path::PathBuf,
        ) -> agent_client_protocol::schema::NewSessionRequest {
            agent_client_protocol::schema::NewSessionRequest::new(cwd)
        }
    }

    fn make_backend() -> AcpBackend {
        AcpBackend::new(
            Arc::new(StubAdapter),
            AcpBackendConfig::default(),
            HashMap::new(),
        )
    }

    #[test]
    fn kind_returns_acp() {
        assert_eq!(make_backend().kind(), "acp");
    }

    #[test]
    fn prompt_channel_cap_is_1024() {
        assert_eq!(PROMPT_CHANNEL_CAP, 1024);
    }

    #[test]
    fn default_config_session_start_timeout_is_30s() {
        let cfg = AcpBackendConfig::default();
        assert_eq!(cfg.session_start_timeout, Duration::from_secs(30));
    }

    #[test]
    fn default_config_read_timeout_is_30s() {
        let cfg = AcpBackendConfig::default();
        assert_eq!(cfg.read_timeout, Duration::from_secs(30));
    }

    #[test]
    fn default_config_tool_progress_timeout_is_60s() {
        let cfg = AcpBackendConfig::default();
        assert_eq!(cfg.tool_progress_timeout, Duration::from_secs(60));
    }

    #[test]
    fn default_config_cancel_grace_is_5s() {
        let cfg = AcpBackendConfig::default();
        assert_eq!(cfg.cancel_grace, Duration::from_secs(5));
    }

    #[test]
    fn default_config_turn_timeout_is_1800s() {
        let cfg = AcpBackendConfig::default();
        assert_eq!(cfg.turn_timeout, Duration::from_secs(1800));
    }

    #[test]
    fn run_turn_no_session_returns_error() {
        let backend = make_backend();
        // No start_session called — run_turn must return SessionStartFailed
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            use crate::tracker::model::Issue;
            use std::sync::atomic::AtomicBool;

            let (tx, _rx) = tokio::sync::mpsc::channel(8);
            let cancelled = Arc::new(AtomicBool::new(false));
            let issue = Issue::default();
            let opts = CliOptions::default();

            let result = backend
                .run_turn(
                    &issue,
                    "hello",
                    None,
                    std::path::Path::new("/tmp"),
                    cancelled,
                    tx,
                    &opts,
                )
                .await;

            assert!(
                matches!(result, Err(SympheoError::SessionStartFailed(_))),
                "expected SessionStartFailed, got {result:?}"
            );
        });
    }

    #[test]
    fn static_permission_hint_bypass_is_allow_all() {
        use crate::agent::acp::adapter::PermissionHint;
        let hint = StubAdapter.static_permission_hint(Some(Permission::BypassPermissions));
        assert_eq!(hint, PermissionHint::AllowAll);
    }

    #[test]
    fn from_service_config_reads_timeouts() {
        use crate::config::typed::ServiceConfig;
        use serde_json::{Map, Value};

        let mut raw = Map::new();
        let mut cli = Map::new();
        cli.insert(
            "session_start_timeout_ms".into(),
            Value::Number(5000.into()),
        );
        cli.insert("read_timeout_ms".into(), Value::Number(2000.into()));
        cli.insert(
            "tool_progress_timeout_ms".into(),
            Value::Number(10000.into()),
        );
        cli.insert("cancel_grace_ms".into(), Value::Number(1000.into()));
        cli.insert("turn_timeout_ms".into(), Value::Number(900000.into()));
        raw.insert("cli".into(), Value::Object(cli));

        let svc = ServiceConfig::new(raw, std::path::PathBuf::from("/tmp"), "".into());
        let cfg = AcpBackendConfig::from_service_config(&svc);

        assert_eq!(cfg.session_start_timeout, Duration::from_millis(5000));
        assert_eq!(cfg.read_timeout, Duration::from_millis(2000));
        assert_eq!(cfg.tool_progress_timeout, Duration::from_millis(10000));
        assert_eq!(cfg.cancel_grace, Duration::from_millis(1000));
        assert_eq!(cfg.turn_timeout, Duration::from_millis(900000));
    }
}
