/// Test-only in-tree fake ACP server.
///
/// Implements the agent-side of the ACP protocol for unit tests and integration
/// tests without requiring a real agent binary.  Behaviour is controlled by a
/// [`Fixture`] struct that declares what the server should do in response to
/// the standard ACP lifecycle messages.
///
/// # Gating
///
/// This module is compiled only in test builds (or when the `fake-acp` feature
/// is enabled) so it never ends up in a release binary.
///
/// # Usage
///
/// ```ignore
/// use crate::agent::acp::fake_server::{Fixture, FakeStopReason};
///
/// let fixture = Fixture::default().with_stop_reason(FakeStopReason::EndTurn);
/// let (client_rx, server_handle) = fixture.run_in_background().await;
/// ```
#[cfg(any(test, feature = "fake-acp"))]
pub use inner::*;

#[cfg(any(test, feature = "fake-acp"))]
mod inner {
    use std::path::PathBuf;
    use std::sync::Arc;

    use agent_client_protocol::schema::{
        AgentCapabilities, ContentBlock, InitializeRequest, InitializeResponse, NewSessionRequest,
        NewSessionResponse, PermissionOption, PromptRequest, PromptResponse, ProtocolVersion,
        RequestPermissionRequest, SessionId, SessionNotification, SessionUpdate, StopReason,
        TextContent, ToolCallId, ToolCallUpdate, ToolCallUpdateFields,
    };
    use agent_client_protocol::{Agent, Client, ConnectionTo, Responder};
    use tokio::io::DuplexStream;
    use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

    // -----------------------------------------------------------------------
    // Fixture types
    // -----------------------------------------------------------------------

    /// Stop reason variants the fake server can return from `session/prompt`.
    #[derive(Debug, Clone, PartialEq, Eq, Default)]
    pub enum FakeStopReason {
        #[default]
        EndTurn,
        MaxTokens,
        MaxTurnRequests,
        Refusal,
        Cancelled,
    }

    impl FakeStopReason {
        fn into_acp(self) -> StopReason {
            match self {
                FakeStopReason::EndTurn => StopReason::EndTurn,
                FakeStopReason::MaxTokens => StopReason::MaxTokens,
                FakeStopReason::MaxTurnRequests => StopReason::MaxTurnRequests,
                FakeStopReason::Refusal => StopReason::Refusal,
                FakeStopReason::Cancelled => StopReason::Cancelled,
            }
        }
    }

    /// Scenario the fake server should play when a `session/prompt` arrives.
    #[derive(Debug, Clone)]
    pub enum PromptScenario {
        /// Emit a simple text update then complete.
        SimpleText {
            text: String,
            stop_reason: FakeStopReason,
        },
        /// Immediately return with the given stop reason (no updates).
        Immediate(FakeStopReason),
        /// Send a `session/request_permission` request to the client then complete.
        RequestPermission {
            options: Vec<PermissionOption>,
            stop_reason: FakeStopReason,
        },
        /// Return an `AUTH_REQUIRED` error.
        AuthRequired,
    }

    impl Default for PromptScenario {
        fn default() -> Self {
            PromptScenario::SimpleText {
                text: "Hello from fake agent".to_string(),
                stop_reason: FakeStopReason::EndTurn,
            }
        }
    }

    /// Declarative fixture that drives the fake server.
    ///
    /// # Fields
    ///
    /// * `protocol_version` — the version the server will advertise in
    ///   `InitializeResponse`.  Set to V0 to trigger a version mismatch.
    /// * `reject_initialize` — if true, close the connection immediately
    ///   without responding to `initialize` (simulates an AUTH_REQUIRED
    ///   failure at handshake time).
    /// * `scenarios` — list of `PromptScenario`s.  Each `session/prompt`
    ///   pops the next scenario from the list; if empty, defaults to
    ///   `PromptScenario::default()`.
    #[derive(Debug, Clone, Default)]
    pub struct Fixture {
        /// ACP version to advertise (default: V1).
        pub protocol_version: Option<ProtocolVersion>,
        /// Close immediately without responding to initialize (AUTH_REQUIRED sim).
        pub reject_initialize: bool,
        /// Per-prompt scenarios (each prompt pops one; empty = default).
        pub scenarios: Vec<PromptScenario>,
    }

    impl Fixture {
        pub fn new() -> Self {
            Self::default()
        }

        /// Advertise a specific protocol version (e.g. V0 to trigger mismatch).
        pub fn with_protocol_version(mut self, v: ProtocolVersion) -> Self {
            self.protocol_version = Some(v);
            self
        }

        /// Simulate an auth-required failure by not responding to initialize.
        pub fn with_reject_initialize(mut self) -> Self {
            self.reject_initialize = true;
            self
        }

        /// Add a prompt scenario.
        pub fn with_scenario(mut self, s: PromptScenario) -> Self {
            self.scenarios.push(s);
            self
        }
    }

    // -----------------------------------------------------------------------
    // Run the fake server
    // -----------------------------------------------------------------------

    /// Spawn a fake server as a Tokio task, returning the client-side stream
    /// half that can be passed to `AcpConnection::connect_streams`.
    ///
    /// The server task completes when the connection closes.
    pub async fn spawn_fake_server(
        fixture: Fixture,
    ) -> (DuplexStream, tokio::task::JoinHandle<()>) {
        let (client_stream, server_stream) = tokio::io::duplex(65536);

        let handle = tokio::task::spawn_local(async move {
            if let Err(e) = run_fake_server(fixture, server_stream).await {
                // In tests, surface connection errors so they aren't silently swallowed.
                eprintln!("[fake_server] error: {e:?}");
            }
        });

        (client_stream, handle)
    }

    async fn run_fake_server(
        fixture: Fixture,
        stream: DuplexStream,
    ) -> Result<(), agent_client_protocol::Error> {
        use std::sync::Mutex;

        let (reader, writer) = tokio::io::split(stream);
        let transport =
            agent_client_protocol::ByteStreams::new(writer.compat_write(), reader.compat());

        let protocol_version = fixture.protocol_version.unwrap_or(ProtocolVersion::V1);
        let reject_initialize = fixture.reject_initialize;
        let scenarios = Arc::new(Mutex::new(fixture.scenarios));

        Agent
            .builder()
            .on_receive_request(
                {
                    let _scenarios = scenarios.clone();
                    move |req: InitializeRequest,
                          responder: Responder<InitializeResponse>,
                          _cx: ConnectionTo<Client>| {
                        let _ = req;
                        // Clone rather than move so the closure stays FnMut (callable
                        // more than once): each invocation gets its own clone while
                        // `protocol_version` remains in the closure's captured environment.
                        let pv = protocol_version.clone();
                        let ri = reject_initialize;
                        async move {
                            if ri {
                                return responder.respond_with_error(
                                    agent_client_protocol::util::internal_error("AUTH_REQUIRED"),
                                );
                            }
                            responder.respond(
                                InitializeResponse::new(pv)
                                    .agent_capabilities(AgentCapabilities::new()),
                            )
                        }
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                |req: NewSessionRequest,
                 responder: Responder<NewSessionResponse>,
                 _cx: ConnectionTo<Client>| {
                    let _ = req;
                    async move { responder.respond(NewSessionResponse::new("fake-session-1")) }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                {
                    let scenarios = scenarios.clone();
                    move |req: PromptRequest,
                          responder: Responder<PromptResponse>,
                          cx: ConnectionTo<Client>| {
                        let session_id = req.session_id.clone();
                        let scenario = {
                            let mut guard = scenarios.lock().unwrap();
                            if guard.is_empty() {
                                PromptScenario::default()
                            } else {
                                guard.remove(0)
                            }
                        };
                        handle_prompt(scenario, session_id, responder, cx)
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(transport)
            .await
    }

    async fn handle_prompt(
        scenario: PromptScenario,
        session_id: SessionId,
        responder: Responder<PromptResponse>,
        cx: ConnectionTo<Client>,
    ) -> Result<(), agent_client_protocol::Error> {
        match scenario {
            PromptScenario::SimpleText { text, stop_reason } => {
                let update = SessionUpdate::AgentMessageChunk(
                    agent_client_protocol::schema::ContentChunk::new(ContentBlock::Text(
                        TextContent::new(text),
                    )),
                );
                cx.send_notification(SessionNotification::new(session_id, update))?;
                responder.respond(PromptResponse::new(stop_reason.into_acp()))
            }
            PromptScenario::Immediate(stop_reason) => {
                responder.respond(PromptResponse::new(stop_reason.into_acp()))
            }
            PromptScenario::RequestPermission {
                options,
                stop_reason,
            } => {
                // Build a minimal ToolCallUpdate to accompany the permission request.
                let tool_call = ToolCallUpdate::new(
                    ToolCallId::new("fake-tc-perm"),
                    ToolCallUpdateFields::default(),
                );
                let perm_req = RequestPermissionRequest::new(session_id, tool_call, options);

                // cx.spawn() runs the future concurrently with the dispatch loop.
                // We cannot use block_task() inline here — it would suspend the
                // dispatch loop task and deadlock (the response can never arrive).
                cx.spawn({
                    let cx = cx.clone();
                    async move {
                        cx.send_request(perm_req).block_task().await?;
                        responder.respond(PromptResponse::new(stop_reason.into_acp()))
                    }
                })?;
                Ok(())
            }
            PromptScenario::AuthRequired => responder
                .respond_with_error(agent_client_protocol::util::internal_error("AUTH_REQUIRED")),
        }
    }

    // -----------------------------------------------------------------------
    // AcpAdapter impl for tests
    // -----------------------------------------------------------------------

    use crate::agent::acp::adapter::{AcpAdapter, default_client_capabilities};
    use agent_client_protocol::schema::{
        ClientCapabilities, NewSessionRequest as AcpNewSessionRequest,
    };
    use std::collections::HashMap;

    /// Minimal [`AcpAdapter`] used in unit tests to validate the trait signature.
    pub struct TestAdapter {
        pub program: String,
    }

    impl TestAdapter {
        pub fn new(program: impl Into<String>) -> Self {
            Self {
                program: program.into(),
            }
        }
    }

    impl AcpAdapter for TestAdapter {
        fn kind(&self) -> &str {
            "test"
        }

        fn spawn_spec(&self, cli_env: &HashMap<String, String>) -> std::process::Command {
            let mut cmd = std::process::Command::new("mise");
            cmd.args(["exec", "--", &self.program]);
            for (k, v) in cli_env {
                cmd.env(k, v);
            }
            cmd
        }

        fn client_capabilities(&self) -> ClientCapabilities {
            default_client_capabilities()
        }

        fn session_new_params(&self, cwd: PathBuf) -> AcpNewSessionRequest {
            AcpNewSessionRequest::new(cwd)
        }
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::agent::acp::adapter::AcpAdapter;
        use agent_client_protocol::schema::{
            PermissionOptionId, PermissionOptionKind, RequestPermissionOutcome,
            RequestPermissionResponse as AcpRequestPermissionResponse, SelectedPermissionOutcome,
        };
        use tokio::task::LocalSet;

        // Validate the TestAdapter satisfies the AcpAdapter trait signature.
        #[test]
        fn test_adapter_satisfies_trait() {
            let adapter = TestAdapter::new("opencode");
            assert_eq!(adapter.kind(), "test");
            let caps = adapter.client_capabilities();
            assert!(!caps.fs.read_text_file);
            assert!(!caps.fs.write_text_file);
            assert!(!caps.terminal);
            assert!(adapter.preflight_check().is_ok());
        }

        #[test]
        fn fixture_default_is_v1() {
            let f = Fixture::default();
            assert!(f.protocol_version.is_none());
            assert!(!f.reject_initialize);
            assert!(f.scenarios.is_empty());
        }

        #[test]
        fn fixture_with_protocol_version_v0() {
            let f = Fixture::new().with_protocol_version(ProtocolVersion::V0);
            assert_eq!(f.protocol_version, Some(ProtocolVersion::V0));
        }

        #[test]
        fn fixture_with_reject_initialize() {
            let f = Fixture::new().with_reject_initialize();
            assert!(f.reject_initialize);
        }

        #[test]
        fn fixture_with_multiple_scenarios() {
            let f = Fixture::new()
                .with_scenario(PromptScenario::Immediate(FakeStopReason::EndTurn))
                .with_scenario(PromptScenario::Immediate(FakeStopReason::Cancelled))
                .with_scenario(PromptScenario::Immediate(FakeStopReason::MaxTokens))
                .with_scenario(PromptScenario::Immediate(FakeStopReason::MaxTurnRequests))
                .with_scenario(PromptScenario::Immediate(FakeStopReason::Refusal));
            assert_eq!(f.scenarios.len(), 5);
        }

        #[test]
        fn all_five_stop_reasons_are_representable() {
            let reasons = [
                FakeStopReason::EndTurn,
                FakeStopReason::MaxTokens,
                FakeStopReason::MaxTurnRequests,
                FakeStopReason::Refusal,
                FakeStopReason::Cancelled,
            ];
            for r in reasons {
                let acp = r.into_acp();
                let _ = acp;
            }
        }

        // --- Integration: connect client to fake server ---

        #[tokio::test(flavor = "current_thread")]
        async fn fake_server_v1_accepts_initialize() {
            let local = LocalSet::new();
            local
                .run_until(async {
                    let fixture = Fixture::new();
                    let (client_stream, _server) = spawn_fake_server(fixture).await;

                    let (reader, writer) = tokio::io::split(client_stream);
                    let transport = agent_client_protocol::ByteStreams::new(
                        writer.compat_write(),
                        reader.compat(),
                    );

                    let result = Client
                        .connect_with(transport, |cx: ConnectionTo<Agent>| async move {
                            let resp = cx
                                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                                .block_task()
                                .await?;
                            assert_eq!(resp.protocol_version, ProtocolVersion::V1);
                            Ok(())
                        })
                        .await;

                    assert!(result.is_ok(), "V1 handshake should succeed: {result:?}");
                })
                .await;
        }

        #[tokio::test(flavor = "current_thread")]
        async fn fake_server_v0_mismatch() {
            let local = LocalSet::new();
            local
                .run_until(async {
                    let fixture = Fixture::new().with_protocol_version(ProtocolVersion::V0);
                    let (client_stream, _server) = spawn_fake_server(fixture).await;

                    let (reader, writer) = tokio::io::split(client_stream);
                    let transport = agent_client_protocol::ByteStreams::new(
                        writer.compat_write(),
                        reader.compat(),
                    );

                    let result = Client
                        .connect_with(transport, |cx: ConnectionTo<Agent>| async move {
                            let resp = cx
                                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                                .block_task()
                                .await?;
                            // Simulate what AcpConnection::connect does: version check
                            crate::agent::acp::connection::check_protocol_version(
                                resp.protocol_version,
                            )
                            .map_err(|e| agent_client_protocol::util::internal_error(e.to_string()))
                        })
                        .await;

                    assert!(result.is_err(), "V0 should trigger an error");
                })
                .await;
        }

        #[tokio::test(flavor = "current_thread")]
        async fn fake_server_reject_initialize_simulates_auth_required() {
            let local = LocalSet::new();
            local
                .run_until(async {
                    let fixture = Fixture::new().with_reject_initialize();
                    let (client_stream, _server) = spawn_fake_server(fixture).await;

                    let (reader, writer) = tokio::io::split(client_stream);
                    let transport = agent_client_protocol::ByteStreams::new(
                        writer.compat_write(),
                        reader.compat(),
                    );

                    let result = Client
                        .connect_with(transport, |cx: ConnectionTo<Agent>| async move {
                            let _resp = cx
                                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                                .block_task()
                                .await?;
                            Ok(())
                        })
                        .await;

                    assert!(result.is_err(), "AUTH_REQUIRED should produce an error");
                })
                .await;
        }

        #[tokio::test(flavor = "current_thread")]
        async fn fake_server_full_lifecycle() {
            use agent_client_protocol::schema::NewSessionRequest as AcpNewSessionRequest;
            let local = LocalSet::new();
            local
                .run_until(async {
                    let fixture = Fixture::new().with_scenario(PromptScenario::SimpleText {
                        text: "result".to_string(),
                        stop_reason: FakeStopReason::EndTurn,
                    });

                    let (client_stream, _server) = spawn_fake_server(fixture).await;
                    let (reader, writer) = tokio::io::split(client_stream);
                    let transport = agent_client_protocol::ByteStreams::new(
                        writer.compat_write(),
                        reader.compat(),
                    );

                    let result = Client
                        .connect_with(transport, |cx: ConnectionTo<Agent>| async move {
                            // initialize
                            let init = cx
                                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                                .block_task()
                                .await?;
                            assert_eq!(init.protocol_version, ProtocolVersion::V1);

                            // session/new
                            let new_sess = cx
                                .send_request(AcpNewSessionRequest::new(PathBuf::from("/tmp")))
                                .block_task()
                                .await?;
                            let session_id = new_sess.session_id;

                            // session/prompt
                            let prompt_resp = cx
                                .send_request(agent_client_protocol::schema::PromptRequest::new(
                                    session_id,
                                    vec![ContentBlock::Text(TextContent::new("hello"))],
                                ))
                                .block_task()
                                .await?;

                            assert_eq!(
                                prompt_resp.stop_reason,
                                StopReason::EndTurn,
                                "should complete with EndTurn"
                            );
                            Ok(())
                        })
                        .await;

                    assert!(result.is_ok(), "full lifecycle should succeed: {result:?}");
                })
                .await;
        }

        #[tokio::test(flavor = "current_thread")]
        async fn fake_server_all_stop_reasons() {
            use agent_client_protocol::schema::NewSessionRequest as AcpNewSessionRequest;
            use agent_client_protocol::schema::PromptRequest;

            let scenarios = vec![
                (FakeStopReason::EndTurn, StopReason::EndTurn),
                (FakeStopReason::MaxTokens, StopReason::MaxTokens),
                (FakeStopReason::MaxTurnRequests, StopReason::MaxTurnRequests),
                (FakeStopReason::Refusal, StopReason::Refusal),
                (FakeStopReason::Cancelled, StopReason::Cancelled),
            ];

            let local = LocalSet::new();
            local
                .run_until(async {
                    for (fake_reason, expected_reason) in scenarios {
                        let fixture =
                            Fixture::new().with_scenario(PromptScenario::Immediate(fake_reason));

                        let (client_stream, _server) = spawn_fake_server(fixture).await;
                        let (reader, writer) = tokio::io::split(client_stream);
                        let transport = agent_client_protocol::ByteStreams::new(
                            writer.compat_write(),
                            reader.compat(),
                        );

                        let result = Client
                            .connect_with(transport, |cx: ConnectionTo<Agent>| {
                                // StopReason is Copy — no .clone() needed
                                let expected = expected_reason;
                                async move {
                                    cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
                                        .block_task()
                                        .await?;
                                    let new_sess = cx
                                        .send_request(AcpNewSessionRequest::new(PathBuf::from(
                                            "/tmp",
                                        )))
                                        .block_task()
                                        .await?;
                                    let prompt_resp = cx
                                        .send_request(PromptRequest::new(
                                            new_sess.session_id,
                                            vec![ContentBlock::Text(TextContent::new("q"))],
                                        ))
                                        .block_task()
                                        .await?;
                                    assert_eq!(
                                        prompt_resp.stop_reason, expected,
                                        "stop reason mismatch"
                                    );
                                    Ok(())
                                }
                            })
                            .await;

                        assert!(result.is_ok(), "stop reason test failed: {result:?}");
                    }
                })
                .await;
        }

        #[test]
        fn fake_server_permission_scenarios_build_correctly() {
            let opts: Vec<PermissionOption> = vec![
                PermissionOption::new(
                    PermissionOptionId::new("allow-once"),
                    "Allow once",
                    PermissionOptionKind::AllowOnce,
                ),
                PermissionOption::new(
                    PermissionOptionId::new("allow-always"),
                    "Allow always",
                    PermissionOptionKind::AllowAlways,
                ),
                PermissionOption::new(
                    PermissionOptionId::new("reject-once"),
                    "Reject once",
                    PermissionOptionKind::RejectOnce,
                ),
            ];

            // Build a fixture with 3 request_permission scenarios (≥3 required by spec).
            let fixture = Fixture::new()
                .with_scenario(PromptScenario::RequestPermission {
                    options: opts.clone(),
                    stop_reason: FakeStopReason::EndTurn,
                })
                .with_scenario(PromptScenario::RequestPermission {
                    options: opts.clone(),
                    stop_reason: FakeStopReason::Cancelled,
                })
                .with_scenario(PromptScenario::RequestPermission {
                    options: opts.clone(),
                    stop_reason: FakeStopReason::EndTurn,
                });

            assert_eq!(fixture.scenarios.len(), 3);
        }

        /// Full round-trip: fake server sends `session/request_permission`; client
        /// responds with `Selected`; server completes the prompt with `EndTurn`.
        #[tokio::test(flavor = "current_thread")]
        async fn fake_server_request_permission_full_flow() {
            use agent_client_protocol::schema::{
                NewSessionRequest as AcpNewSessionRequest, PromptRequest,
            };

            let opts = vec![
                PermissionOption::new(
                    PermissionOptionId::new("allow-once"),
                    "Allow once",
                    PermissionOptionKind::AllowOnce,
                ),
                PermissionOption::new(
                    PermissionOptionId::new("reject-once"),
                    "Reject once",
                    PermissionOptionKind::RejectOnce,
                ),
            ];

            let fixture = Fixture::new().with_scenario(PromptScenario::RequestPermission {
                options: opts,
                stop_reason: FakeStopReason::EndTurn,
            });

            let local = LocalSet::new();
            local
                .run_until(async {
                    let (client_stream, _server) = spawn_fake_server(fixture).await;
                    let (reader, writer) = tokio::io::split(client_stream);
                    let transport = agent_client_protocol::ByteStreams::new(
                        writer.compat_write(),
                        reader.compat(),
                    );

                    let result = Client
                        .builder()
                        .on_receive_request(
                            |req: RequestPermissionRequest,
                             responder: Responder<AcpRequestPermissionResponse>,
                             _cx: ConnectionTo<Agent>| {
                                async move {
                                    // Select the first option the server offered.
                                    let option_id = req.options[0].option_id.clone();
                                    responder.respond(AcpRequestPermissionResponse::new(
                                        RequestPermissionOutcome::Selected(
                                            SelectedPermissionOutcome::new(option_id),
                                        ),
                                    ))
                                }
                            },
                            agent_client_protocol::on_receive_request!(),
                        )
                        .connect_with(transport, |cx: ConnectionTo<Agent>| async move {
                            cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
                                .block_task()
                                .await?;
                            let new_sess = cx
                                .send_request(AcpNewSessionRequest::new(PathBuf::from("/tmp")))
                                .block_task()
                                .await?;
                            let prompt_resp = cx
                                .send_request(PromptRequest::new(
                                    new_sess.session_id,
                                    vec![ContentBlock::Text(TextContent::new("hello"))],
                                ))
                                .block_task()
                                .await?;
                            assert_eq!(
                                prompt_resp.stop_reason,
                                StopReason::EndTurn,
                                "prompt should complete with EndTurn after permission exchange"
                            );
                            Ok(())
                        })
                        .await;

                    assert!(
                        result.is_ok(),
                        "request_permission round-trip should succeed: {result:?}"
                    );
                })
                .await;
        }

        /// Four permission modes × three scenarios each = 12 round-trips.
        ///
        /// Validates that the `RequestPermission` scenario correctly sends the
        /// `session/request_permission` request and that the client's response is
        /// received before the prompt completes.
        #[tokio::test(flavor = "current_thread")]
        async fn fake_server_request_permission_four_modes_three_scenarios() {
            use agent_client_protocol::schema::{
                NewSessionRequest as AcpNewSessionRequest, PromptRequest,
            };

            let modes_outcomes = [
                (PermissionOptionKind::AllowOnce, StopReason::EndTurn),
                (PermissionOptionKind::AllowAlways, StopReason::EndTurn),
                (PermissionOptionKind::RejectOnce, StopReason::Cancelled),
                (PermissionOptionKind::RejectAlways, StopReason::Cancelled),
            ];

            let local = LocalSet::new();
            local
                .run_until(async {
                    for (kind, expected_stop_reason) in modes_outcomes {
                        for scenario_idx in 0..3usize {
                            let opts = vec![PermissionOption::new(
                                PermissionOptionId::new(format!("opt-{scenario_idx}")),
                                "Option",
                                kind,
                            )];
                            let fake_stop = match expected_stop_reason {
                                StopReason::EndTurn => FakeStopReason::EndTurn,
                                _ => FakeStopReason::Cancelled,
                            };
                            let fixture =
                                Fixture::new().with_scenario(PromptScenario::RequestPermission {
                                    options: opts,
                                    stop_reason: fake_stop,
                                });

                            let (client_stream, _server) = spawn_fake_server(fixture).await;
                            let (reader, writer) = tokio::io::split(client_stream);
                            let transport = agent_client_protocol::ByteStreams::new(
                                writer.compat_write(),
                                reader.compat(),
                            );

                            let result = Client
                                .builder()
                                .on_receive_request(
                                    |req: RequestPermissionRequest,
                                     responder: Responder<AcpRequestPermissionResponse>,
                                     _cx: ConnectionTo<Agent>| {
                                        async move {
                                            let option_id = req.options[0].option_id.clone();
                                            responder.respond(AcpRequestPermissionResponse::new(
                                                RequestPermissionOutcome::Selected(
                                                    SelectedPermissionOutcome::new(option_id),
                                                ),
                                            ))
                                        }
                                    },
                                    agent_client_protocol::on_receive_request!(),
                                )
                                .connect_with(transport, |cx: ConnectionTo<Agent>| async move {
                                    cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
                                        .block_task()
                                        .await?;
                                    let new_sess = cx
                                        .send_request(AcpNewSessionRequest::new(PathBuf::from(
                                            "/tmp",
                                        )))
                                        .block_task()
                                        .await?;
                                    let prompt_resp = cx
                                        .send_request(PromptRequest::new(
                                            new_sess.session_id,
                                            vec![ContentBlock::Text(TextContent::new("q"))],
                                        ))
                                        .block_task()
                                        .await?;
                                    assert_eq!(
                                        prompt_resp.stop_reason, expected_stop_reason,
                                        "kind={kind:?} scenario={scenario_idx}"
                                    );
                                    Ok(())
                                })
                                .await;

                            assert!(
                                result.is_ok(),
                                "mode={kind:?} scenario={scenario_idx} failed: {result:?}"
                            );
                        }
                    }
                })
                .await;
        }
    }
}
