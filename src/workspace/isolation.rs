//! SPEC §15.5 hardening (implementation-defined, opt-in by design).
//!
//! Per-worker filesystem isolation for the local execution backend:
//!
//! - HOME and XDG_*_HOME are scoped under `<workspace>/.sympheo-home/...` so the
//!   coding-agent CLI cannot read or write the host operator's `~/.config/opencode`,
//!   `~/.local/share/opencode`, etc.
//! - PATH is restricted to a minimal whitelist so the agent inherits only the
//!   binaries needed to run (bash, coreutils, git, gh, opencode if installed
//!   system-wide). When `mise` is detected on the host, its binary dir and
//!   shims dir are auto-prepended so any tool the operator installs via mise
//!   (gh, opencode, bun, ...) becomes resolvable inside the worker without
//!   per-tool configuration. Operators MAY override the whole PATH via
//!   `cli.env.PATH` in `WORKFLOW.md`.
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

/// Locations that mise uses on the host. Both must be on the worker PATH:
/// the shims dispatch to the right tool versions, and each shim invokes
/// `mise` itself to resolve which version to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MisePaths {
    /// Directory containing the `mise` binary.
    pub bin_dir: PathBuf,
    /// Directory containing the per-tool shims (gh, opencode, bun, ...).
    pub shims_dir: PathBuf,
}

/// Locate `mise` on the host. Returns `None` when the binary is absent.
///
/// Resolution order for the data dir (where `shims/` lives) follows mise's
/// own convention: `MISE_DATA_DIR` → `XDG_DATA_HOME/mise` →
/// `$HOME/.local/share/mise`.
pub fn find_mise_paths() -> Option<MisePaths> {
    let bin = find_in_host_path("mise")?;
    let bin_dir = bin.parent()?.to_path_buf();
    let data_dir = mise_data_dir()?;
    Some(MisePaths {
        bin_dir,
        shims_dir: data_dir.join("shims"),
    })
}

fn mise_data_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("MISE_DATA_DIR")
        && !d.is_empty()
    {
        return Some(PathBuf::from(d));
    }
    if let Ok(x) = std::env::var("XDG_DATA_HOME")
        && !x.is_empty()
    {
        return Some(PathBuf::from(x).join("mise"));
    }
    let home = std::env::var("HOME").ok()?;
    Some(
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("mise"),
    )
}

fn find_in_host_path(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(bin);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Subdirectories created under `<workspace>/.sympheo-home/` for the scoped
/// HOME and XDG_*_HOME env vars. Stable on disk so reuse across turns is cheap.
pub const HOME_SUBDIR: &str = ".sympheo-home";
pub const CONFIG_SUBDIR: &str = ".sympheo-home/.config";
pub const DATA_SUBDIR: &str = ".sympheo-home/.local/share";
pub const CACHE_SUBDIR: &str = ".sympheo-home/.cache";
pub const STATE_SUBDIR: &str = ".sympheo-home/.local/state";

/// Minimal PATH used when the operator hasn't provided one via `cli.env.PATH`.
/// Includes the system bin dirs so bash, git, and opencode (if installed
/// system-wide) are discoverable, plus `~/.local/bin` mapped INTO the scoped
/// HOME. When `mise` is available on the host its shims dir and binary dir
/// are prepended so mise-managed tools (gh, opencode, bun, ...) resolve
/// without per-tool configuration.
fn default_path(home: &Path, mise: Option<&MisePaths>) -> String {
    let mut entries: Vec<String> = Vec::new();
    if let Some(m) = mise {
        entries.push(m.shims_dir.display().to_string());
        entries.push(m.bin_dir.display().to_string());
    }
    entries.push(home.join(".local").join("bin").display().to_string());
    entries.extend([
        "/usr/local/sbin".to_string(),
        "/usr/local/bin".to_string(),
        "/usr/sbin".to_string(),
        "/usr/bin".to_string(),
        "/sbin".to_string(),
        "/bin".to_string(),
    ]);
    entries.join(":")
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

    // 3. Default PATH (with mise paths prepended when available so the
    //    worker can resolve mise-managed CLIs by name).
    let mise = find_mise_paths();
    env.insert("PATH".to_string(), default_path(&home, mise.as_ref()));

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
    fn test_default_path_with_mise_prepends_shims_and_bin() {
        let home = PathBuf::from("/tmp/ws/.sympheo-home");
        let mise = MisePaths {
            bin_dir: PathBuf::from("/opt/mise/bin"),
            shims_dir: PathBuf::from("/opt/mise/shims"),
        };
        let path = default_path(&home, Some(&mise));
        let entries: Vec<&str> = path.split(':').collect();
        assert_eq!(
            entries[0], "/opt/mise/shims",
            "shims must be the first PATH entry so mise-managed CLIs win"
        );
        assert_eq!(
            entries[1], "/opt/mise/bin",
            "mise binary dir comes second so shims can call back into mise"
        );
        assert!(entries.contains(&"/usr/bin"));
    }

    #[test]
    fn test_default_path_without_mise_omits_shims() {
        let home = PathBuf::from("/tmp/ws/.sympheo-home");
        let path = default_path(&home, None);
        assert!(!path.contains("shims"));
        assert!(path.starts_with(&home.join(".local").join("bin").display().to_string()));
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
