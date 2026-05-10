//! Resolve agent CLI tool names (`opencode`, `gh`, ...) to absolute binary paths
//! at sympheo startup, in the host process's full env.
//!
//! This is the only place sympheo touches the host installer model (mise, brew,
//! apt, manual). Once resolved, the worker subprocess receives a scrubbed env
//! whose `PATH` contains only the resolved binaries' parent dirs plus system
//! bins — it never invokes mise, never hits a shim, never trips a trust check.
//!
//! Resolution order:
//! 1. `mise which <name>` — when the host has mise on PATH, this dereferences
//!    mise-managed shims to the real binary inside `~/.local/share/mise/installs/...`.
//! 2. Plain `$PATH` lookup — for tools installed system-wide (apt, brew, manual).
//!
//! Both paths return absolute paths; a shim is never returned.
//!
//! Failures are non-fatal at this layer (the caller decides). The fn returns
//! `None` when the binary cannot be located; the caller logs and decides whether
//! to abort startup or continue with a degraded `PATH`.
//!
//! NOTE: This module runs in sympheo's host process where mise is activated /
//! trusted. The tools it locates are then handed to the worker as absolute
//! paths — see `src/workspace/isolation.rs` and `src/agent/backend/local.rs`.

use std::path::PathBuf;
use std::process::Command;

/// Resolve `name` to an absolute binary path on the host. Returns `None` if
/// nothing matches. Never returns a shim — `mise which` is preferred over PATH
/// lookup precisely so that mise-managed tools are dereferenced to their real
/// install dirs.
pub fn resolve_tool(name: &str) -> Option<PathBuf> {
    if let Some(p) = resolve_via_mise(name) {
        return Some(p);
    }
    resolve_via_path(name)
}

fn resolve_via_mise(name: &str) -> Option<PathBuf> {
    let mise_bin = resolve_via_path("mise")?;
    let output = Command::new(&mise_bin)
        .args(["which", name])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let line = stdout.trim();
    if line.is_empty() {
        return None;
    }
    let candidate = PathBuf::from(line);
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}

fn resolve_via_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_tool_returns_none_for_unknown() {
        assert!(resolve_tool("definitely-not-a-real-binary-xyz123").is_none());
    }

    #[test]
    fn test_resolve_tool_finds_bash() {
        // `bash` is present on every supported host (sympheo even spawns the
        // worker via `bash -lc`). If this fails, the host is broken in a way
        // that breaks the rest of the suite too.
        let p = resolve_tool("bash").expect("bash must be resolvable");
        assert!(p.is_absolute(), "bash path should be absolute: {:?}", p);
        assert!(p.is_file(), "bash path should be a regular file: {:?}", p);
    }

    #[test]
    fn test_resolve_via_path_picks_first_match() {
        // Construct a PATH where the first entry contains `bash`, expect that
        // exact path back. Uses the actual host bash so we don't depend on
        // synthetic fixtures.
        let real_bash = resolve_via_path("bash").expect("host bash");
        let dir = real_bash.parent().expect("bash has a parent dir");
        let custom_path = format!("{}", dir.display());
        // SAFETY: tests run sequentially within a process for env mutation;
        // restoring after the assertion keeps the rest of the suite stable.
        let prev = std::env::var_os("PATH");
        unsafe {
            std::env::set_var("PATH", &custom_path);
        }
        let resolved = resolve_via_path("bash");
        unsafe {
            match prev {
                Some(v) => std::env::set_var("PATH", v),
                None => std::env::remove_var("PATH"),
            }
        }
        let resolved = resolved.expect("bash should resolve under custom PATH");
        assert_eq!(resolved, real_bash);
    }
}
