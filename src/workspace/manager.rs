use crate::config::typed::ServiceConfig;
use crate::error::SympheoError;
use crate::tracker::model::WorkspaceInfo;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tokio::time::{Duration, timeout};

pub struct WorkspaceManager {
    root: PathBuf,
    hook_timeout: Duration,
    git_adapter: Option<Arc<dyn crate::git::GitAdapter>>,
    repo_url: Option<String>,
}

impl WorkspaceManager {
    pub fn new(config: &ServiceConfig) -> Result<Self, SympheoError> {
        let root = config.workspace_root()?;
        Ok(Self {
            root,
            hook_timeout: Duration::from_millis(config.hook_timeout_ms()),
            git_adapter: None,
            repo_url: config.workspace_repo_url(),
        })
    }

    pub fn set_git_adapter(&mut self, adapter: Arc<dyn crate::git::GitAdapter>) {
        self.git_adapter = Some(adapter);
    }

    pub fn git_adapter(&self) -> &Option<Arc<dyn crate::git::GitAdapter>> {
        &self.git_adapter
    }

    /// SPEC §4.2 + §9.2 + §15.2: workspace key = identifier with any character outside
    /// `[A-Za-z0-9._]` replaced by `-`. Examples:
    ///   sympheo#42      -> sympheo-42
    ///   ABC-123         -> ABC-123        (hyphen kept; outside [.A-Za-z0-9_] but spec sample)
    ///   feat/new_thing  -> feat-new_thing
    ///
    /// Note: SPEC §4.2 lists `[A-Za-z0-9._]` strictly; the GitHub example
    /// `sympheo#42 -> sympheo-42` shows hyphen as the replacement char. The Linear example
    /// `ABC-123 -> ABC-123` shows hyphen survives (because spec text says "not in" the
    /// allowed set, but the result preserves the hyphen). We interpret the conformance
    /// requirement as: replace any character outside `[A-Za-z0-9._-]` with `-`,
    /// matching both example mappings.
    pub fn sanitize_identifier(identifier: &str) -> String {
        identifier
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect()
    }

    pub fn workspace_path(&self, identifier: &str) -> PathBuf {
        let key = Self::sanitize_identifier(identifier);
        self.root.join(&key)
    }

    pub async fn create_or_reuse(
        &self,
        identifier: &str,
        after_create_hook: Option<&str>,
    ) -> Result<WorkspaceInfo, SympheoError> {
        let path = self.workspace_path(identifier);
        let created_now = if !path.exists() {
            tokio::fs::create_dir_all(&path).await.map_err(|e| {
                SympheoError::WorkspaceError(format!("failed to create workspace dir: {e}"))
            })?;
            true
        } else {
            false
        };

        if created_now {
            if let (Some(adapter), Some(url)) = (&self.git_adapter, &self.repo_url) {
                crate::git::GitAdapter::clone(&**adapter, url, &path)
                    .await
                    .map_err(|e| SympheoError::WorkspaceError(format!("git clone failed: {e}")))?;
            } else if let Some(script) = after_create_hook {
                self.run_hook("after_create", script, &path).await?;
            }
        }

        Ok(WorkspaceInfo {
            path,
            workspace_key: Self::sanitize_identifier(identifier),
            created_now,
        })
    }

    pub async fn run_hook(&self, name: &str, script: &str, cwd: &Path) -> Result<(), SympheoError> {
        tracing::info!(hook = name, cwd = %cwd.display(), "running workspace hook");
        let mut child = Command::new("bash")
            .arg("-lc")
            .arg(script)
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SympheoError::HookFailed(format!("spawn failed for {name}: {e}")))?;

        let result = timeout(self.hook_timeout, child.wait()).await;
        match result {
            Ok(Ok(status)) => {
                if status.success() {
                    Ok(())
                } else {
                    Err(SympheoError::HookFailed(format!(
                        "hook {name} exited with {status}"
                    )))
                }
            }
            Ok(Err(e)) => Err(SympheoError::HookFailed(format!(
                "hook {name} wait error: {e}"
            ))),
            Err(_) => {
                let _ = child.kill().await;
                Err(SympheoError::HookFailed(format!(
                    "hook {name} timed out after {:?}",
                    self.hook_timeout
                )))
            }
        }
    }

    pub async fn remove_workspace(&self, identifier: &str, before_remove: Option<&str>) {
        let path = self.workspace_path(identifier);
        if path.exists() {
            if let Some(script) = before_remove
                && let Err(e) = self.run_hook("before_remove", script, &path).await
            {
                tracing::warn!(error = %e, "before_remove hook failed");
            }
            if let Err(e) = tokio::fs::remove_dir_all(&path).await {
                tracing::warn!(path = %path.display(), error = %e, "failed to remove workspace");
            }
        }
    }

    pub fn validate_inside_root(&self, path: &Path) -> Result<(), SympheoError> {
        let root = self
            .root
            .canonicalize()
            .unwrap_or_else(|_| self.root.clone());
        let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if !target.starts_with(&root) {
            return Err(SympheoError::WorkspaceError(
                "workspace path is outside root".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_config_with_root(root: PathBuf) -> ServiceConfig {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut workspace = serde_json::Map::<String, serde_json::Value>::new();
        workspace.insert(
            "root".into(),
            serde_json::Value::String(root.to_string_lossy().to_string()),
        );
        raw.insert("workspace".into(), serde_json::Value::Object(workspace));
        ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into())
    }

    fn unique_tmp(suffix: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("sympheo_test_{}_{}", suffix, ts))
    }

    #[test]
    fn test_sanitize_identifier_basic() {
        // SPEC §4.2 examples
        assert_eq!(
            WorkspaceManager::sanitize_identifier("sympheo#42"),
            "sympheo-42"
        );
        assert_eq!(WorkspaceManager::sanitize_identifier("ABC-123"), "ABC-123");
        assert_eq!(
            WorkspaceManager::sanitize_identifier("feat/new_thing"),
            "feat-new_thing"
        );
        assert_eq!(
            WorkspaceManager::sanitize_identifier("bug: crash!"),
            "bug--crash-"
        );
    }

    #[test]
    fn test_sanitize_identifier_preserves_dots() {
        assert_eq!(WorkspaceManager::sanitize_identifier("v1.2.3"), "v1.2.3");
    }

    #[test]
    fn test_sanitize_identifier_replaces_with_hyphen() {
        // any char outside [A-Za-z0-9._-] becomes '-'
        assert_eq!(WorkspaceManager::sanitize_identifier("a@b/c d"), "a-b-c-d");
    }

    #[test]
    fn test_workspace_path() {
        let tmp = unique_tmp("ws");
        let config = test_config_with_root(tmp.clone());
        let mgr = WorkspaceManager::new(&config).unwrap();
        let path = mgr.workspace_path("ISSUE-42");
        assert_eq!(path, tmp.join("ISSUE-42"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_create_or_reuse_new() {
        let tmp = unique_tmp("new");
        let _ = std::fs::remove_dir_all(&tmp);
        let config = test_config_with_root(tmp.clone());
        let mgr = WorkspaceManager::new(&config).unwrap();
        let info = mgr.create_or_reuse("NEW-1", None).await.unwrap();
        assert!(info.path.exists());
        assert!(info.created_now);
        assert_eq!(info.workspace_key, "NEW-1");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_create_or_reuse_existing() {
        let tmp = unique_tmp("exist");
        let _ = std::fs::remove_dir_all(&tmp);
        let config = test_config_with_root(tmp.clone());
        let mgr = WorkspaceManager::new(&config).unwrap();
        let info1 = mgr.create_or_reuse("EXIST-1", None).await.unwrap();
        assert!(info1.created_now);
        let info2 = mgr.create_or_reuse("EXIST-1", None).await.unwrap();
        assert!(!info2.created_now);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_create_or_reuse_with_after_create_hook() {
        let tmp = unique_tmp("hook");
        let _ = std::fs::remove_dir_all(&tmp);
        let config = test_config_with_root(tmp.clone());
        let mgr = WorkspaceManager::new(&config).unwrap();
        let info = mgr
            .create_or_reuse("HOOK-1", Some("echo hello > created"))
            .await
            .unwrap();
        assert!(info.created_now);
        assert!(info.path.join("created").exists());
        let contents = std::fs::read_to_string(info.path.join("created")).unwrap();
        assert_eq!(contents.trim(), "hello");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_run_hook_success() {
        let tmp = unique_tmp("hook_ok");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let config = test_config_with_root(tmp.clone());
        let mgr = WorkspaceManager::new(&config).unwrap();
        let result = mgr.run_hook("test", "echo hello", &tmp).await;
        assert!(result.is_ok());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_run_hook_failure() {
        let tmp = unique_tmp("hook_fail");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let config = test_config_with_root(tmp.clone());
        let mgr = WorkspaceManager::new(&config).unwrap();
        let result = mgr.run_hook("test", "exit 1", &tmp).await;
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_run_hook_timeout() {
        let tmp = unique_tmp("hook_timeout");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut workspace = serde_json::Map::<String, serde_json::Value>::new();
        workspace.insert(
            "root".into(),
            serde_json::Value::String(tmp.to_string_lossy().to_string()),
        );
        raw.insert("workspace".into(), serde_json::Value::Object(workspace));
        let mut hooks = serde_json::Map::<String, serde_json::Value>::new();
        hooks.insert("timeout_ms".into(), serde_json::Value::Number(100.into()));
        raw.insert("hooks".into(), serde_json::Value::Object(hooks));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let mgr = WorkspaceManager::new(&config).unwrap();
        let result = mgr.run_hook("test", "sleep 5", &tmp).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("timed out"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_remove_workspace() {
        let tmp = unique_tmp("rem");
        let _ = std::fs::remove_dir_all(&tmp);
        let config = test_config_with_root(tmp.clone());
        let mgr = WorkspaceManager::new(&config).unwrap();
        let info = mgr.create_or_reuse("REM-1", None).await.unwrap();
        assert!(info.path.exists());
        mgr.remove_workspace("REM-1", None).await;
        assert!(!info.path.exists());
    }

    #[tokio::test]
    async fn test_remove_workspace_with_before_remove_hook() {
        let tmp = unique_tmp("remhook");
        let _ = std::fs::remove_dir_all(&tmp);
        let config = test_config_with_root(tmp.clone());
        let mgr = WorkspaceManager::new(&config).unwrap();
        let info = mgr.create_or_reuse("REM-2", None).await.unwrap();
        assert!(info.path.exists());
        mgr.remove_workspace("REM-2", Some("echo bye")).await;
        assert!(!info.path.exists());
    }

    #[test]
    fn test_validate_inside_root_ok() {
        let tmp = unique_tmp("validate");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let config = test_config_with_root(tmp.clone());
        let mgr = WorkspaceManager::new(&config).unwrap();
        mgr.validate_inside_root(&tmp.join("sub")).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_create_or_reuse_dir_create_error() {
        let tmp = unique_tmp("readonly_parent");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let readonly = tmp.join("readonly");
        std::fs::create_dir_all(&readonly).unwrap();
        let mut perms = std::fs::metadata(&readonly).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&readonly, perms).unwrap();

        let config = test_config_with_root(readonly.clone());
        let mgr = WorkspaceManager::new(&config).unwrap();
        let result = mgr.create_or_reuse("FAIL-1", None).await;
        assert!(result.is_err());

        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_remove_workspace_hook_failure() {
        let tmp = unique_tmp("remhookfail");
        let _ = std::fs::remove_dir_all(&tmp);
        let config = test_config_with_root(tmp.clone());
        let mgr = WorkspaceManager::new(&config).unwrap();
        let info = mgr.create_or_reuse("REM-FAIL", None).await.unwrap();
        assert!(info.path.exists());
        mgr.remove_workspace("REM-FAIL", Some("exit 1")).await;
        assert!(!info.path.exists());
    }

    #[tokio::test]
    async fn test_remove_workspace_rm_error() {
        let tmp = unique_tmp("rmfail");
        let _ = std::fs::remove_dir_all(&tmp);
        let config = test_config_with_root(tmp.clone());
        let mgr = WorkspaceManager::new(&config).unwrap();
        let info = mgr.create_or_reuse("RM-FAIL", None).await.unwrap();
        assert!(info.path.exists());
        // Create a file inside
        std::fs::write(info.path.join("file.txt"), "data").unwrap();
        // Make workspace read-only to block removal of contents
        let mut perms = std::fs::metadata(&info.path).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&info.path, perms).unwrap();
        mgr.remove_workspace("RM-FAIL", None).await;
        // The workspace should still exist because removal failed
        assert!(info.path.exists());
        // Cleanup: restore permissions
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&info.path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_validate_inside_root_fail() {
        let tmp = unique_tmp("validate_fail");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let config = test_config_with_root(tmp.clone());
        let mgr = WorkspaceManager::new(&config).unwrap();
        let result = mgr.validate_inside_root(Path::new("/etc"));
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
