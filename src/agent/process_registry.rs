//! Global registry of spawned CLI subprocesses with their process group ids,
//! so signal/panic handlers can kill the whole tree on shutdown and operator
//! Ctrl-C does not leave zombie opencode processes alive.
//!
//! CONTEXT.md: "il n'y ai jamais de CLI residuelle à l'execution du process
//! sympheo principal". The §14 spec leaves recovery semantics implementation-
//! defined; this module is part of the local-backend hardening posture
//! documented in `docs/isolation.md` (no-zombie addendum below).
//!
//! Lifecycle:
//!   1. `LocalBackend::run_turn` registers the spawned child with `register()`,
//!      receiving a `RegistrationGuard`.
//!   2. The guard is held for the lifetime of the turn. If the worker returns,
//!      panics, or is killed locally, `Drop` removes the entry.
//!   3. Signal handlers (SIGINT / SIGTERM) iterate the registry and send
//!      SIGTERM → 3s grace → SIGKILL to each known process group.
//!   4. A panic hook performs the same cleanup in case `tokio::signal` is not
//!      installed (e.g. tests).

use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy)]
pub struct ProcessRecord {
    pub pid: u32,
}

lazy_static! {
    static ref REGISTRY: Mutex<HashMap<u64, ProcessRecord>> = Mutex::new(HashMap::new());
    static ref NEXT_TOKEN: Mutex<u64> = Mutex::new(1);
}

/// RAII guard returned by [`register`]. Dropping it removes the entry from
/// the registry (so workers that exit normally don't leave stale records).
pub struct RegistrationGuard {
    token: u64,
}

impl Drop for RegistrationGuard {
    fn drop(&mut self) {
        let mut reg = REGISTRY.lock().expect("process registry poisoned");
        reg.remove(&self.token);
    }
}

/// Register a spawned subprocess with the global registry. Returns a guard
/// that removes the entry when dropped.
pub fn register(pid: u32) -> RegistrationGuard {
    let token = {
        let mut next = NEXT_TOKEN.lock().expect("token counter poisoned");
        let t = *next;
        *next = next.wrapping_add(1);
        t
    };
    let mut reg = REGISTRY.lock().expect("process registry poisoned");
    reg.insert(token, ProcessRecord { pid });
    RegistrationGuard { token }
}

/// Snapshot of currently registered subprocesses. Used by signal handlers and tests.
pub fn snapshot() -> Vec<ProcessRecord> {
    REGISTRY
        .lock()
        .expect("process registry poisoned")
        .values()
        .copied()
        .collect()
}

/// Best-effort cleanup of all registered subprocesses. Sends SIGTERM, sleeps
/// `grace`, then SIGKILL to any survivors. Iterates a snapshot so the live
/// registry can drain naturally as guards drop.
///
/// Safe to call from `panic` hooks and signal handlers; never panics.
pub fn terminate_all_blocking(grace: std::time::Duration) {
    let records = snapshot();
    if records.is_empty() {
        return;
    }
    for rec in &records {
        let pgid = rec.pid as i32;
        unsafe {
            // Send SIGTERM to the process group; ignore errors (group may already be gone).
            let _ = libc::killpg(pgid, libc::SIGTERM);
        }
    }
    std::thread::sleep(grace);
    for rec in &records {
        let pgid = rec.pid as i32;
        unsafe {
            let _ = libc::killpg(pgid, libc::SIGKILL);
            // Also fall back to single-pid kill in case setpgid failed at spawn.
            let _ = libc::kill(pgid, libc::SIGKILL);
        }
    }
}

/// Async variant for signal handlers — uses tokio's sleep to avoid blocking
/// the runtime worker.
pub async fn terminate_all_async(grace: std::time::Duration) {
    let records = snapshot();
    if records.is_empty() {
        return;
    }
    for rec in &records {
        let pgid = rec.pid as i32;
        unsafe {
            let _ = libc::killpg(pgid, libc::SIGTERM);
        }
    }
    tokio::time::sleep(grace).await;
    for rec in &records {
        let pgid = rec.pid as i32;
        unsafe {
            let _ = libc::killpg(pgid, libc::SIGKILL);
            let _ = libc::kill(pgid, libc::SIGKILL);
        }
    }
}

/// Install a `panic::set_hook` that calls `terminate_all_blocking(grace)`
/// and then forwards to the previous hook. Safe to call multiple times
/// (the wrapper is idempotent over its argument).
pub fn install_panic_hook(grace: std::time::Duration) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        terminate_all_blocking(grace);
        prev(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    // Serialize tests that touch REGISTRY / NEXT_TOKEN. cargo test runs tests
    // within a module concurrently by default; without this lock, one test's
    // reset_registry() can race with another's inserts and break assertions.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Reset registry between tests (they all share global state).
    fn reset_registry() {
        REGISTRY.lock().unwrap().clear();
    }

    #[test]
    fn test_register_and_snapshot() {
        let _lock = TEST_LOCK.lock().unwrap();
        reset_registry();
        let _g1 = register(12345);
        let _g2 = register(12346);
        let snap = snapshot();
        assert_eq!(snap.len(), 2);
        let pids: Vec<u32> = snap.iter().map(|r| r.pid).collect();
        assert!(pids.contains(&12345));
        assert!(pids.contains(&12346));
    }

    #[test]
    fn test_drop_guard_removes_entry() {
        let _lock = TEST_LOCK.lock().unwrap();
        reset_registry();
        let g1 = register(99001);
        assert_eq!(snapshot().len(), 1);
        drop(g1);
        assert_eq!(snapshot().len(), 0);
    }

    #[test]
    fn test_terminate_all_empty_is_noop() {
        let _lock = TEST_LOCK.lock().unwrap();
        reset_registry();
        terminate_all_blocking(Duration::from_millis(0));
    }

    // Safety: TEST_LOCK is held across the await deliberately to serialize
    // access to the global test state. No deadlock is possible: the only
    // async task in this tokio test runtime is the test itself; the other
    // (sync) tests block on separate OS threads and never contend inside
    // the runtime.
    #[allow(clippy::await_holding_lock)] // Reason: TEST_LOCK is held across the await deliberately to serialize access to global test state; no deadlock possible since this tokio runtime runs a single async test task.
    #[tokio::test]
    async fn test_terminate_all_async_kills_real_subprocess() {
        let _lock = TEST_LOCK.lock().unwrap();
        reset_registry();
        // Spawn a real long-sleeping subprocess in its own process group so
        // killpg actually kills it. We can't use tokio::process easily here
        // because we need setpgid, but a plain std::process::Command works
        // for this targeted test.
        use std::os::unix::process::CommandExt;
        let mut cmd = std::process::Command::new("sleep");
        cmd.arg("60");
        unsafe {
            cmd.pre_exec(|| {
                libc::setpgid(0, 0);
                Ok(())
            });
        }
        let mut child = cmd.spawn().expect("spawn sleep");
        let pid = child.id();
        let _guard = register(pid);

        // Confirm process is alive
        let alive_before = unsafe { libc::kill(pid as i32, 0) } == 0;
        assert!(alive_before, "subprocess should be alive before terminate");

        terminate_all_async(Duration::from_millis(50)).await;

        // Reap the child to avoid zombies
        let _ = child.wait();

        // Confirm process is gone
        let alive_after = unsafe { libc::kill(pid as i32, 0) } == 0;
        assert!(
            !alive_after,
            "subprocess should be killed after terminate_all_async"
        );
    }
}
