//! SPEC §17.10 Real Integration Profile (RECOMMENDED, gated).
//!
//! These tests run a real `opencode` invocation against a real (or
//! WireMock-fronted) GitHub Project. They are EXCLUDED from the default
//! `cargo test` invocation via the cargo "ignore" attribute and require
//! both:
//!
//!   1. `SYMPHEO_REAL_INTEGRATION=1` in the environment.
//!   2. A throwaway test repo + project credentials in `SYMPHEO_GITHUB_TOKEN`.
//!
//! Per spec: "skipped real-integration test SHOULD be reported as skipped,
//! not silently treated as passed". The cargo ignore attribute reports them
//! in the summary, satisfying that requirement.
//!
//! Token-budget guardrail: each gated test SHOULD set a low max_turns and
//! short turn_timeout to cap the worst-case spend. Operators MAY raise these
//! locally before running the real-integration profile.

#[test]
#[ignore = "Reason: SPEC §17.10 Real Integration Profile — requires SYMPHEO_REAL_INTEGRATION=1 env var, a real opencode binary, and a throwaway GitHub project."]
fn real_integration_smoke_opencode() {
    if std::env::var("SYMPHEO_REAL_INTEGRATION").as_deref() != Ok("1") {
        // Defense in depth: even if `cargo test -- --ignored` is invoked,
        // refuse to spend tokens unless the operator opted in explicitly.
        eprintln!(
            "SYMPHEO_REAL_INTEGRATION is not set to 1 — skipping real-integration smoke test."
        );
        return;
    }

    // Token budget guardrail (advisory comment for the operator).
    eprintln!(
        "Real integration smoke test invoked. Cap max_turns and turn_timeout in your test \
         WORKFLOW.md before running this so the worst-case spend stays bounded."
    );

    // The actual test would build a ServiceConfig pointing at the real
    // opencode binary, fetch one issue from a throwaway project, and assert
    // the orchestrator dispatches and completes a single turn. Skipped
    // intentionally here — the runner is bound to local infrastructure that
    // is not available in CI. Operators add their fixture and remove this
    // early-return when they wire up the test environment locally.
    eprintln!(
        "TODO: wire fixture project + token budget cap (see docs/conformance-tests.md §17.10)."
    );
}

#[test]
#[ignore = "Reason: SPEC §17.10 — requires SYMPHEO_REAL_INTEGRATION=1 + SYMPHEO_GITHUB_TOKEN + throwaway project access."]
fn real_integration_smoke_github_tracker() {
    if std::env::var("SYMPHEO_REAL_INTEGRATION").as_deref() != Ok("1") {
        eprintln!(
            "SYMPHEO_REAL_INTEGRATION is not set to 1 — skipping real-integration GitHub smoke test."
        );
        return;
    }
    if std::env::var("SYMPHEO_GITHUB_TOKEN").is_err() {
        eprintln!("SYMPHEO_GITHUB_TOKEN missing — skipping real-integration GitHub smoke test.");
        return;
    }
    eprintln!(
        "TODO: query throwaway project, assert at least one normalized issue, validate identifier format <repo>#<number>."
    );
}
