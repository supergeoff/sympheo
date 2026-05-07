use crate::config::typed::ServiceConfig;
use crate::error::SymphonyError;
use crate::tracker::model::WorkspaceInfo;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

pub struct WorkspaceManager {
    root: PathBuf,
    hook_timeout: Duration,
}

impl WorkspaceManager {
    pub fn new(config: &ServiceConfig) -> Result<Self, SymphonyError> {
        let root = config.workspace_root()?;
        Ok(Self {
            root,
            hook_timeout: Duration::from_millis(config.hook_timeout_ms()),
        })
    }

    pub fn sanitize_identifier(identifier: &str) -> String {
        identifier
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                    c
                } else {
                    '_'
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
    ) -> Result<WorkspaceInfo, SymphonyError> {
        let path = self.workspace_path(identifier);
        let created_now = if !path.exists() {
            tokio::fs::create_dir_all(&path).await.map_err(|e| {
                SymphonyError::WorkspaceError(format!("failed to create workspace dir: {e}"))
            })?;
            true
        } else {
            false
        };

        if created_now {
            if let Some(script) = after_create_hook {
                self.run_hook("after_create", script, &path).await?;
            }
        }

        Ok(WorkspaceInfo {
            path,
            workspace_key: Self::sanitize_identifier(identifier),
            created_now,
        })
    }

    pub async fn run_hook(
        &self,
        name: &str,
        script: &str,
        cwd: &Path,
    ) -> Result<(), SymphonyError> {
        tracing::info!(hook = name, cwd = %cwd.display(), "running workspace hook");
        let mut child = Command::new("bash")
            .arg("-lc")
            .arg(script)
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SymphonyError::HookFailed(format!("spawn failed for {name}: {e}")))?;

        let result = timeout(self.hook_timeout, child.wait()).await;
        match result {
            Ok(Ok(status)) => {
                if status.success() {
                    Ok(())
                } else {
                    Err(SymphonyError::HookFailed(format!(
                        "hook {name} exited with {status}"
                    )))
                }
            }
            Ok(Err(e)) => Err(SymphonyError::HookFailed(format!(
                "hook {name} wait error: {e}"
            ))),
            Err(_) => {
                let _ = child.kill().await;
                Err(SymphonyError::HookFailed(format!(
                    "hook {name} timed out after {:?}",
                    self.hook_timeout
                )))
            }
        }
    }

    pub async fn remove_workspace(&self, identifier: &str, before_remove: Option<&str>) {
        let path = self.workspace_path(identifier);
        if path.exists() {
            if let Some(script) = before_remove {
                if let Err(e) = self.run_hook("before_remove", script, &path).await {
                    tracing::warn!(error = %e, "before_remove hook failed");
                }
            }
            if let Err(e) = tokio::fs::remove_dir_all(&path).await {
                tracing::warn!(path = %path.display(), error = %e, "failed to remove workspace");
            }
        }
    }

    pub fn validate_inside_root(&self, path: &Path) -> Result<(), SymphonyError> {
        let root = self.root.canonicalize().unwrap_or_else(|_| self.root.clone());
        let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if !target.starts_with(&root) {
            return Err(SymphonyError::WorkspaceError(
                "workspace path is outside root".into(),
            ));
        }
        Ok(())
    }
}
