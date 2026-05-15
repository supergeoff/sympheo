//! Subprocess entry-point for ACP integration tests (lot 5).
//!
//! Reads `FAKE_ACP_SCENARIO` from the environment, builds the corresponding
//! [`Fixture`], and serves the ACP protocol on stdin/stdout until the client
//! closes the connection or the fake's long-sleep timer expires.
//!
//! Compiled only when the `fake-acp` feature is enabled; never included in
//! release builds.
use sympheo::agent::acp::fake_server::{
    FakeStopReason, Fixture, PromptScenario,
};

fn main() {
    let scenario = std::env::var("FAKE_ACP_SCENARIO").unwrap_or_default();
    let fixture = build_fixture(&scenario);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, sympheo::agent::acp::fake_server::serve_on_io(fixture));
}

fn build_fixture(scenario: &str) -> Fixture {
    use agent_client_protocol::schema::{
        PermissionOption, PermissionOptionId, PermissionOptionKind, ProtocolVersion,
    };

    match scenario {
        // Lifecycle / stop-reason scenarios ----------------------------------
        "lifecycle_end_turn" => Fixture::new().with_scenario(PromptScenario::SimpleText {
            text: "hello from fake agent".to_string(),
            stop_reason: FakeStopReason::EndTurn,
        }),

        "stop_reason_max_tokens" => {
            Fixture::new().with_scenario(PromptScenario::Immediate(FakeStopReason::MaxTokens))
        }
        "stop_reason_max_turn_requests" => Fixture::new()
            .with_scenario(PromptScenario::Immediate(FakeStopReason::MaxTurnRequests)),
        "stop_reason_refusal" => {
            Fixture::new().with_scenario(PromptScenario::Immediate(FakeStopReason::Refusal))
        }
        "stop_reason_cancelled" => {
            Fixture::new().with_scenario(PromptScenario::Immediate(FakeStopReason::Cancelled))
        }

        // Cancel cooperative: send 3 trailing updates then Cancelled ---------
        "cancel_trailing_3" => {
            Fixture::new().with_scenario(PromptScenario::CancelTrailingUpdates {
                trailing_count: 3,
                stop_reason: FakeStopReason::Cancelled,
            })
        }

        // Silence (for read / tool-progress timeout tests) ------------------
        "silence" => Fixture::new().with_scenario(PromptScenario::Silence),

        // Session-start timeout: freeze on initialize -----------------------
        "silence_on_init" => Fixture::new().with_silence_on_initialize(),

        // Protocol mismatch -------------------------------------------------
        "protocol_v0" => Fixture::new().with_protocol_version(ProtocolVersion::V0),

        // AUTH_REQUIRED at initialize ---------------------------------------
        "auth_required_init" => Fixture::new().with_reject_initialize(),

        // AUTH_REQUIRED returned inside session/prompt ----------------------
        "auth_required_prompt" => {
            Fixture::new().with_scenario(PromptScenario::AuthRequired)
        }

        // request_permission: BypassPermissions mode (allow-once option) ----
        "perm_bypass_read" => {
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
            Fixture::new().with_scenario(PromptScenario::RequestPermission {
                options: opts,
                stop_reason: FakeStopReason::EndTurn,
            })
        }

        // request_permission: Plan mode with Execute (should reject) ---------
        "perm_plan_execute" => {
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
            Fixture::new().with_scenario(PromptScenario::RequestPermission {
                options: opts,
                stop_reason: FakeStopReason::EndTurn,
            })
        }

        // request_permission: Default mode with Read (should allow) ----------
        "perm_default_read" => {
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
            Fixture::new().with_scenario(PromptScenario::RequestPermission {
                options: opts,
                stop_reason: FakeStopReason::EndTurn,
            })
        }

        // request_permission: AcceptEdits mode with Edit (should allow) ------
        "perm_accept_edits_edit" => {
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
            Fixture::new().with_scenario(PromptScenario::RequestPermission {
                options: opts,
                stop_reason: FakeStopReason::EndTurn,
            })
        }

        // Fallback: identity test with EndTurn --------------------------------
        _ => Fixture::new().with_scenario(PromptScenario::Immediate(FakeStopReason::EndTurn)),
    }
}
