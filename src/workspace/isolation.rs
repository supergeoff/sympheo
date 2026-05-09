//! SPEC §15.5 hardening (implementation-defined, opt-in by design).
//!
//! Per-worker filesystem isolation for the local execution backend:
//!
//! - HOME and XDG_*_HOME are scoped under `<workspace>/.sympheo-home/...` so the
//!   coding-agent CLI cannot read or write the host operator's `~/.config/opencode`,
//!   `~/.local/share/opencode`, etc.
//! - PATH is restricted to a minimal whitelist so the agent inherits only the
//!   binaries needed to run (bash, coreutils, git, gh, opencode if installed
//!   system-wide). Operators MAY extend this whitelist via `cli.env.PATH` in
//!   `WORKFLOW.md`.
//! - All other env vars are scrubbed except a small whitelist of locale / TTY
//!   variables that are safe and useful (LANG, LC_*, TERM, TZ, USER, LOGNAME).
//! - Any explicit `cli.env` entries declared in WORKFLOW.md (§5.3.6) override
//!   the defaults.
//!
//! The launching subprocess still uses `bash -lc <cli.command>` per §5.3.6;
//! since HOME points inside the workspace, bash's login-profile loading
//! processes the empty `<workspace>/.sympheo-home` rather than the operator's
//! profile.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Subdirectories created under `<workspace>/.sympheo-home/` for the scoped
/// HOME and XDG_*_HOME env vars. Stable on disk so reuse across turns is cheap.
pub const HOME_SUBDIR: &str = ".sympheo-home";
pub const CONFIG_SUBDIR: &str = ".sympheo-home/.config";
pub const DATA_SUBDIR: &str = ".sympheo-home/.local/share";
pub const CACHE_SUBDIR: &str = ".sympheo-home/.cache";
pub const STATE_SUBDIR: &str = ".sympheo-home/.local/state";

/// Minimal PATH used when the operator hasn't provided one via `cli.env.PATH`.
/// Includes the system bin dirs so bash, git, and opencode (if installed
/// system-wide) are discoverable, plus `~/.local/bin` mapped INTO the
/// scoped HOME.
fn default_path(home: &Path) -> String {
    let local_bin = home.join(".local").join("bin");
    [
        local_bin.display().to_string(),
        "/usr/local/sbin".to_string(),
        "/usr/local/bin".to_string(),
        "/usr/sbin".to_string(),
        "/usr/bin".to_string(),
        "/sbin".to_string(),
        "/bin".to_string(),
    ]
    .join(":")
}

/// Allowlist of env vars passed through from the host process verbatim
/// (when set). Locale / terminal-related only — no credentials, no PATH.
const HOST_PASSTHROUGH_VARS: &[&str] = &[
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_CTYPE",
    "LC_COLLATE",
    "LC_MESSAGES",
    "LC_NUMERIC",
    "LC_TIME",
    "TERM",
    "TZ",
    "USER",
    "LOGNAME",
];

/// Build the env map to apply to a CLI subprocess launched in `workspace_path`.
///
/// Order of precedence (low → high):
/// 1. Whitelisted host passthrough (LANG, TZ, etc.)
/// 2. Sympheo-managed scoping: HOME + XDG_*_HOME pointing under the workspace
/// 3. Default minimal PATH
/// 4. `cli.env` from WORKFLOW.md (§5.3.6) — operator override
pub fn build_isolated_env(
    workspace_path: &Path,
    cli_env_overrides: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut env = HashMap::new();

    // 1. Host passthrough whitelist
    for var in HOST_PASSTHROUGH_VARS {
        if let Ok(val) = std::env::var(var) {
            env.insert((*var).to_string(), val);
        }
    }

    // 2. Sympheo-managed scoping
    let home = workspace_path.join(HOME_SUBDIR);
    let config = workspace_path.join(CONFIG_SUBDIR);
    let data = workspace_path.join(DATA_SUBDIR);
    let cache = workspace_path.join(CACHE_SUBDIR);
    let state = workspace_path.join(STATE_SUBDIR);
    env.insert("HOME".to_string(), home.display().to_string());
    env.insert("XDG_CONFIG_HOME".to_string(), config.display().to_string());
    env.insert("XDG_DATA_HOME".to_string(), data.display().to_string());
    env.insert("XDG_CACHE_HOME".to_string(), cache.display().to_string());
    env.insert("XDG_STATE_HOME".to_string(), state.display().to_string());

    // 3. Default PATH
    env.insert("PATH".to_string(), default_path(&home));

    // 4. Operator overrides — always wins
    for (k, v) in cli_env_overrides {
        env.insert(k.clone(), v.clone());
    }

    env
}

/// Create the `.sympheo-home` subtree under the workspace if absent.
/// Idempotent — safe to call on every turn.
pub async fn ensure_isolated_home(workspace_path: &Path) -> std::io::Result<PathBuf> {
    let home = workspace_path.join(HOME_SUBDIR);
    for sub in &[
        HOME_SUBDIR,
        CONFIG_SUBDIR,
        DATA_SUBDIR,
        CACHE_SUBDIR,
        STATE_SUBDIR,
        ".sympheo-home/.local/bin",
    ] {
        let p = workspace_path.join(sub);
        tokio::fs::create_dir_all(&p).await?;
    }
    Ok(home)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_tmp(suffix: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("sympheo_iso_test_{}_{}", suffix, ts))
    }

    #[test]
    fn test_build_isolated_env_scopes_home_xdg() {
        let ws = PathBuf::from("/tmp/wstest");
        let env = build_isolated_env(&ws, &HashMap::new());
        assert_eq!(
            env.get("HOME"),
            Some(&"/tmp/wstest/.sympheo-home".to_string())
        );
        assert_eq!(
            env.get("XDG_CONFIG_HOME"),
            Some(&"/tmp/wstest/.sympheo-home/.config".to_string())
        );
        assert_eq!(
            env.get("XDG_DATA_HOME"),
            Some(&"/tmp/wstest/.sympheo-home/.local/share".to_string())
        );
        assert_eq!(
            env.get("XDG_CACHE_HOME"),
            Some(&"/tmp/wstest/.sympheo-home/.cache".to_string())
        );
        assert_eq!(
            env.get("XDG_STATE_HOME"),
            Some(&"/tmp/wstest/.sympheo-home/.local/state".to_string())
        );
    }

    #[test]
    fn test_build_isolated_env_default_path_minimal() {
        let ws = PathBuf::from("/tmp/wstest");
        let env = build_isolated_env(&ws, &HashMap::new());
        let path = env.get("PATH").expect("PATH should be set");
        assert!(path.contains("/usr/bin"));
        assert!(path.contains("/usr/local/bin"));
        assert!(path.contains(".sympheo-home/.local/bin"));
        // No leakage of arbitrary host PATH entries (best-effort: we only set
        // the minimal whitelist here).
    }

    #[test]
    fn test_build_isolated_env_cli_env_overrides_path() {
        let ws = PathBuf::from("/tmp/wstest");
        let mut overrides = HashMap::new();
        overrides.insert("PATH".to_string(), "/custom/bin".to_string());
        overrides.insert("ANTHROPIC_API_KEY".to_string(), "sk-test".to_string());
        let env = build_isolated_env(&ws, &overrides);
        assert_eq!(env.get("PATH"), Some(&"/custom/bin".to_string()));
        assert_eq!(env.get("ANTHROPIC_API_KEY"), Some(&"sk-test".to_string()));
        // Sympheo-managed HOME still set
        assert!(env.contains_key("HOME"));
    }

    #[test]
    fn test_build_isolated_env_no_credentials_passthrough() {
        unsafe {
            std::env::set_var("ANTHROPIC_API_KEY", "should-not-leak");
            std::env::set_var("AWS_ACCESS_KEY_ID", "should-not-leak-too");
        }
        let ws = PathBuf::from("/tmp/wstest");
        let env = build_isolated_env(&ws, &HashMap::new());
        assert!(!env.contains_key("ANTHROPIC_API_KEY"));
        assert!(!env.contains_key("AWS_ACCESS_KEY_ID"));
        unsafe {
            std::env::remove_var("ANTHROPIC_API_KEY");
            std::env::remove_var("AWS_ACCESS_KEY_ID");
        }
    }

    #[tokio::test]
    async fn test_ensure_isolated_home_creates_subtree() {
        let tmp = unique_tmp("home");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let home = ensure_isolated_home(&tmp).await.unwrap();
        assert!(home.exists());
        assert!(tmp.join(CONFIG_SUBDIR).exists());
        assert!(tmp.join(DATA_SUBDIR).exists());
        assert!(tmp.join(CACHE_SUBDIR).exists());
        assert!(tmp.join(STATE_SUBDIR).exists());
        assert!(tmp.join(".sympheo-home/.local/bin").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_ensure_isolated_home_is_idempotent() {
        let tmp = unique_tmp("home_idem");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        ensure_isolated_home(&tmp).await.unwrap();
        ensure_isolated_home(&tmp).await.unwrap();
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
