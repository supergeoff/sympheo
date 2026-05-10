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

// Workflow that passes `validate_for_dispatch` and skips the mise check.
//
// Three knobs are doing important work here:
//  - `daytona.enabled: true` + a non-empty `api_key` skips the host mise
//    discovery check, so this test does not require mise on the runner.
//  - `endpoint: http://nonexistent-test-host.invalid` makes the startup
//    terminal cleanup's GitHub call fail fast on DNS rather than hanging
//    on a TCP timeout (RFC 6761 reserves `.invalid`). The cleanup is
//    demoted to a background retry, and main proceeds to install the
//    signal handler.
//  - `cli.command: mock-cli` + `cli.options.script: ...` lets the mock
//    backend init succeed without spawning a subprocess. The script file
//    itself is never read because no turn runs in this test.
#[cfg(unix)]
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

/// Spawn sympheo against `DISPATCHABLE_WORKFLOW`, poll its stderr until the
/// binary has reached the post-cleanup state (signal handler is installed
/// shortly after), then send `signal_num` and wait for graceful exit.
///
/// Returns `(exit_code, stderr_text)` so callers can also assert on the log
/// line that triggered the signal. Polling stderr for a deterministic readiness
/// marker replaces the prior fixed-1500-ms sleep, which was a flaky pattern on
/// slow CI runners. SIGINT and SIGTERM share this helper so the host-lifecycle
/// invariant (`std::process::exit(0)` after either signal) is exercised
/// symmetrically.
#[cfg(unix)]
fn run_until_ready_then_signal(label: &str, signal_num: i32) -> (Option<i32>, String) {
    use std::process::Stdio;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    let dir = unique_tmp(label);
    let workflow_path = dir.join("WORKFLOW.md");
    std::fs::write(&workflow_path, DISPATCHABLE_WORKFLOW).expect("write workflow");

    let mut child = sympheo_bin()
        .current_dir(&dir)
        .env("RUST_LOG", "info")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sympheo");

    let pid = child.id() as i32;

    // Drain BOTH stdout and stderr concurrently — `tracing_subscriber::fmt()`
    // defaults to stdout, while typed startup errors surface on stderr. We
    // need both: stderr to surface unexpected failures, stdout to detect the
    // readiness marker. If we leave either pipe undrained the OS buffer fills
    // and the child stalls.
    let stdout_pipe = child.stdout.take().expect("child stdout");
    let stderr_pipe = child.stderr.take().expect("child stderr");
    let collected: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let ready_pair: Arc<(Mutex<bool>, std::sync::Condvar)> =
        Arc::new((Mutex::new(false), std::sync::Condvar::new()));

    fn drain_into<R: std::io::Read + Send + 'static>(
        reader: R,
        tag: &'static str,
        collected: Arc<Mutex<String>>,
        ready: Arc<(Mutex<bool>, std::sync::Condvar)>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let buf = std::io::BufReader::new(reader);
            use std::io::BufRead;
            for line in buf.lines().map_while(Result::ok) {
                {
                    let mut store = collected.lock().unwrap();
                    store.push_str(tag);
                    store.push_str(": ");
                    store.push_str(&line);
                    store.push('\n');
                }
                // Markers emitted AFTER startup validation passed and the
                // terminal cleanup has either completed or been demoted to
                // background retry. Either marker proves the main thread has
                // moved past the synchronous startup phase; the
                // SIGTERM/SIGINT spawn that follows is scheduled within
                // microseconds on tokio's runtime.
                if line.contains("startup terminal cleanup completed")
                    || line.contains("scheduling background retry")
                {
                    let (lock, cvar) = &*ready;
                    let mut r = lock.lock().unwrap();
                    *r = true;
                    cvar.notify_all();
                }
            }
        })
    }

    let _drain_out = drain_into(stdout_pipe, "stdout", collected.clone(), ready_pair.clone());
    let _drain_err = drain_into(stderr_pipe, "stderr", collected.clone(), ready_pair.clone());

    // Bounded wait for the readiness marker. On a slow CI runner DNS may take
    // a moment to fail; 10s is generous.
    let (lock, cvar) = &*ready_pair;
    let mut ready = lock.lock().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !*ready {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let (g, _) = cvar
            .wait_timeout(ready, deadline - now)
            .expect("condvar wait");
        ready = g;
    }
    let became_ready = *ready;
    drop(ready);

    // If the binary exited before reaching readiness, fail loudly with the
    // captured stderr so the operator can see why.
    if !became_ready {
        if let Some(early) = child.try_wait().expect("try_wait") {
            let _ = std::fs::remove_dir_all(&dir);
            let stderr = collected.lock().unwrap().clone();
            panic!("binary exited before reaching readiness: status={early:?}\nstderr:\n{stderr}");
        }
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
        let stderr = collected.lock().unwrap().clone();
        panic!("binary did not reach readiness within 10s\nstderr:\n{stderr}");
    }

    // Brief grace window so the tokio-spawned signal handler task runs at
    // least once and registers SIGTERM/SIGINT before we deliver the signal.
    std::thread::sleep(Duration::from_millis(150));

    let rc = unsafe { libc::kill(pid, signal_num) };
    assert_eq!(rc, 0, "libc::kill({signal_num}) failed");

    // Bounded wait for graceful exit. The handler calls `exit(0)` after a 3s
    // child-process drain, so 8s is plenty.
    let deadline = Instant::now() + Duration::from_secs(8);
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(s) => break s,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_dir_all(&dir);
                let stderr = collected.lock().unwrap().clone();
                panic!("binary did not exit within 8s of signal {signal_num}\nstderr:\n{stderr}");
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };

    let _ = std::fs::remove_dir_all(&dir);
    let stderr = collected.lock().unwrap().clone();
    (status.code(), stderr)
}

#[cfg(unix)]
#[test]
fn binary_exits_zero_on_normal_startup_and_sigterm() {
    // §17.9: "exits with success when the application starts and shuts down
    // normally" via SIGTERM. The handler in `src/main.rs` registers both
    // SIGINT and SIGTERM and, on either, drains spawned children and calls
    // `std::process::exit(0)`.
    let (code, stderr) = run_until_ready_then_signal("sigterm", libc::SIGTERM);
    assert_eq!(
        code,
        Some(0),
        "expected exit 0 after graceful SIGTERM, got {code:?}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("SIGTERM received"),
        "stderr must record SIGTERM receipt; got:\n{stderr}"
    );
}

#[cfg(unix)]
#[test]
fn binary_exits_zero_on_normal_startup_and_sigint() {
    // §17.9: graceful shutdown coverage MUST be symmetric for SIGINT, since
    // the handler in `src/main.rs` registers both. A regression that only
    // wired SIGTERM would pass the SIGTERM test and silently break Ctrl-C.
    let (code, stderr) = run_until_ready_then_signal("sigint", libc::SIGINT);
    assert_eq!(
        code,
        Some(0),
        "expected exit 0 after graceful SIGINT, got {code:?}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("SIGINT received"),
        "stderr must record SIGINT receipt; got:\n{stderr}"
    );
}
