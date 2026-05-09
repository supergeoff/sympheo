//! SPEC §17.9 — CLI and Host Lifecycle.
//!
//! Drives the actual `sympheo` binary built by Cargo (`CARGO_BIN_EXE_sympheo`)
//! and asserts the six host-lifecycle behaviors:
//!
//! - positional workflow path argument is accepted
//! - default to `./WORKFLOW.md` when no path is provided
//! - missing explicit path is a clean nonzero exit
//! - missing default `./WORKFLOW.md` is a clean nonzero exit
//! - YAML front matter parse failure surfaces cleanly on stderr
//! - normal startup followed by SIGTERM exits 0
//!
//! Tests are hermetic: each one uses an isolated tmpdir, the bundled
//! `mock-cli` adapter, and (where the binary needs to enter its main loop)
//! a tracker endpoint pointed at `127.0.0.1:1` so any GitHub call fails
//! immediately with `connection refused` rather than hitting the network.

use std::path::PathBuf;
use std::process::Command;

fn sympheo_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sympheo"))
}

fn unique_tmp(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "sympheo_cli_lifecycle_{}_{}_{}",
        label,
        std::process::id(),
        nanos
    ));
    std::fs::create_dir_all(&dir).expect("create tmpdir");
    dir
}

#[test]
fn binary_accepts_positional_workflow_path() {
    // §17.9: "accepts a positional workflow path argument (`path-to-WORKFLOW.md`)".
    //
    // Sanity: pass an explicit path to a missing file in a tmpdir whose name
    // is unique. The binary must surface that exact path in its error output —
    // proving the positional argument was honored (and not, e.g., silently
    // replaced by the cwd default).
    let dir = unique_tmp("positional");
    let explicit = dir.join("custom-name.md");

    let out = sympheo_bin()
        .arg(&explicit)
        .current_dir(&dir)
        .output()
        .expect("spawn sympheo");

    let _ = std::fs::remove_dir_all(&dir);

    assert!(!out.status.success(), "expected nonzero exit");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("custom-name.md"),
        "stderr should reference the positional path; got:\n{stderr}"
    );
}

#[test]
fn binary_defaults_to_workflow_md_when_no_arg() {
    // §17.9: "uses `./WORKFLOW.md` when no workflow path argument is provided".
    //
    // Run with no arg in a clean tmpdir where `WORKFLOW.md` does not exist.
    // The binary must mention `WORKFLOW.md` (the default) in its error path —
    // never some other file name.
    let dir = unique_tmp("default");

    let out = sympheo_bin()
        .current_dir(&dir)
        .output()
        .expect("spawn sympheo");

    let _ = std::fs::remove_dir_all(&dir);

    assert!(!out.status.success(), "expected nonzero exit");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("WORKFLOW.md"),
        "stderr should reference the default WORKFLOW.md; got:\n{stderr}"
    );
}

#[test]
fn binary_errors_on_nonexistent_explicit_path() {
    // §17.9: "errors on nonexistent explicit workflow path".
    let dir = unique_tmp("missing-explicit");
    let missing = dir.join("does-not-exist.md");

    let out = sympheo_bin()
        .arg(&missing)
        .current_dir(&dir)
        .output()
        .expect("spawn sympheo");

    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        !out.status.success(),
        "binary must exit nonzero on missing explicit path"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("does-not-exist.md") || stderr.contains("missing workflow"),
        "stderr should explain the missing workflow; got:\n{stderr}"
    );
}

#[test]
fn binary_errors_on_missing_default_workflow_md() {
    // §17.9: "errors on ... missing default `./WORKFLOW.md`".
    let dir = unique_tmp("missing-default");
    assert!(!dir.join("WORKFLOW.md").exists());

    let out = sympheo_bin()
        .current_dir(&dir)
        .output()
        .expect("spawn sympheo");

    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        !out.status.success(),
        "binary must exit nonzero when no WORKFLOW.md is present"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("WORKFLOW.md") || stderr.contains("missing workflow"),
        "stderr should mention the missing WORKFLOW.md; got:\n{stderr}"
    );
}

#[test]
fn binary_surfaces_startup_failure_cleanly() {
    // §17.9: "surfaces startup failure cleanly" + "exits nonzero when startup
    // fails". An unclosed YAML front matter is the simplest way to provoke a
    // typed startup error without touching tracker config or the network.
    let dir = unique_tmp("invalid");
    let workflow_path = dir.join("WORKFLOW.md");
    std::fs::write(&workflow_path, "---\ntracker: kind\nDo work").expect("write workflow");

    let out = sympheo_bin()
        .current_dir(&dir)
        .output()
        .expect("spawn sympheo");

    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        !out.status.success(),
        "binary must exit nonzero on workflow parse failure"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("workflow parse error")
            || stderr.contains("WorkflowParseError")
            || stderr.contains("front matter"),
        "stderr should surface the parse failure; got:\n{stderr}"
    );
    // The binary must surface the typed error, not panic.
    assert!(
        !stderr.contains("panicked at"),
        "binary must not panic on a typed startup error; got:\n{stderr}"
    );
}

#[cfg(unix)]
#[test]
fn binary_exits_zero_on_normal_startup_and_sigterm() {
    // §17.9: "exits with success when the application starts and shuts down
    // normally". Drive a fully dispatchable workflow, wait briefly for the
    // main loop to be reachable, then send SIGTERM. The signal handler in
    // `src/main.rs` calls `std::process::exit(0)` after draining child
    // processes, so a graceful SIGTERM must yield exit code 0.
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    // Workflow that passes `validate_for_dispatch` and skips the mise check.
    //
    // Three knobs are doing important work here:
    //  - `daytona.enabled: true` + a non-empty `api_key` skips the host mise
    //    discovery check, so this test does not require mise on the runner.
    //  - `endpoint: http://nonexistent-test-host.invalid` makes the startup
    //    terminal cleanup's GitHub call fail fast on DNS rather than hanging
    //    on a TCP timeout (RFC 6761 reserves `.invalid`). The cleanup is
    //    demoted to a background retry, and main proceeds to install the
    //    SIGTERM handler.
    //  - `cli.command: mock-cli` + `cli.options.script: ...` lets the mock
    //    backend init succeed without spawning a subprocess. The script file
    //    itself is never read because no turn runs in this test.
    const DISPATCHABLE_WORKFLOW: &str = r#"---
tracker:
  kind: github
  project_slug: test-org/test-repo
  project_number: 1
  api_key: dummy-token-not-used
  endpoint: http://nonexistent-test-host.invalid
cli:
  command: mock-cli
  options:
    script: ./mock-script.yaml
daytona:
  enabled: true
  api_key: dummy-daytona-key
polling:
  interval_ms: 60000
---
Test prompt {{ issue.title }}
"#;

    let dir = unique_tmp("sigterm");
    let workflow_path = dir.join("WORKFLOW.md");
    std::fs::write(&workflow_path, DISPATCHABLE_WORKFLOW).expect("write workflow");

    let mut child = sympheo_bin()
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sympheo");

    let pid = child.id() as i32;

    // Give the binary time to: parse the workflow, fail the startup terminal
    // cleanup against 127.0.0.1:1 (immediate connection refused), demote it
    // to the background retry loop, and install the signal handler.
    std::thread::sleep(Duration::from_millis(1500));

    // If the binary already exited (e.g., dispatch validation failed), the
    // SIGTERM-shutdown invariant we want to test cannot be exercised — fail
    // loudly with the captured streams so the failure mode is obvious.
    if let Some(early) = child.try_wait().expect("try_wait") {
        let _ = std::fs::remove_dir_all(&dir);
        let stdout = child
            .stdout
            .take()
            .map(|mut s| {
                use std::io::Read;
                let mut buf = String::new();
                let _ = s.read_to_string(&mut buf);
                buf
            })
            .unwrap_or_default();
        let stderr = child
            .stderr
            .take()
            .map(|mut s| {
                use std::io::Read;
                let mut buf = String::new();
                let _ = s.read_to_string(&mut buf);
                buf
            })
            .unwrap_or_default();
        panic!(
            "binary exited before SIGTERM could be sent: status={:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            early
        );
    }

    // SIGTERM via libc — already a project dependency.
    let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
    assert_eq!(rc, 0, "libc::kill(SIGTERM) failed");

    // Bounded wait for graceful exit. The signal handler calls
    // `std::process::exit(0)` after a 3s child-process drain, so 8s is plenty.
    let deadline = Instant::now() + Duration::from_secs(8);
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(s) => break s,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_dir_all(&dir);
                panic!("binary did not exit within 8s of SIGTERM");
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };

    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        status.code(),
        Some(0),
        "expected exit 0 after graceful SIGTERM, got {:?}",
        status.code()
    );
}
