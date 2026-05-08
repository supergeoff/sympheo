use crate::error::SympheoError;
use crate::git::adapter::{CommitInfo, GitAdapter, GitStatus, MergeStrategy};
use async_trait::async_trait;
use std::path::Path;
use tokio::process::Command;

pub struct LocalGitAdapter;

impl LocalGitAdapter {
    pub fn new() -> Self {
        Self
    }

    async fn run_git(
        &self,
        path: &Path,
        args: &[&str],
    ) -> Result<(String, String), SympheoError> {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(path);
        for a in args {
            cmd.arg(a);
        }
        cmd.env("GIT_TERMINAL_PROMPT", "0");

        let output = cmd
            .output()
            .await
            .map_err(|e| SympheoError::GitError(format!("git spawn failed: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            return Err(SympheoError::GitError(format!(
                "git {} failed: {} (stdout: {})",
                args.join(" "),
                stderr.trim(),
                stdout.trim()
            )));
        }

        Ok((stdout, stderr))
    }
}

impl Default for LocalGitAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl GitAdapter for LocalGitAdapter {
    async fn clone(&self, url: &str, path: &Path) -> Result<(), SympheoError> {
        let parent = path.parent().ok_or_else(|| {
            SympheoError::GitError(format!("path {} has no parent", path.display()))
        })?;
        let name = path.file_name().ok_or_else(|| {
            SympheoError::GitError(format!("path {} has no file name", path.display()))
        })?;

        let mut cmd = Command::new("git");
        cmd.arg("clone")
            .arg(url)
            .arg(name)
            .current_dir(parent)
            .env("GIT_TERMINAL_PROMPT", "0");

        let output = cmd
            .output()
            .await
            .map_err(|e| SympheoError::GitError(format!("git clone spawn failed: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SympheoError::GitError(format!(
                "git clone failed: {}",
                stderr.trim()
            )));
        }
        Ok(())
    }

    async fn checkout_branch(
        &self,
        path: &Path,
        branch: &str,
        create: bool,
    ) -> Result<(), SympheoError> {
        let mut args = vec!["checkout"];
        if create {
            args.push("-b");
        }
        args.push(branch);
        let _ = self.run_git(path, &args).await?;
        Ok(())
    }

    async fn commit(
        &self,
        path: &Path,
        message: &str,
        files: &[&str],
    ) -> Result<String, SympheoError> {
        for file in files {
            let _ = self.run_git(path, &["add", file]).await?;
        }
        let _ = self.run_git(path, &["commit", "-m", message]).await?;
        let (stdout, _) = self.run_git(path, &["rev-parse", "HEAD"]).await?;
        Ok(stdout.trim().to_string())
    }

    async fn push(&self, path: &Path, remote: &str, branch: &str) -> Result<(), SympheoError> {
        let _ = self
            .run_git(path, &["push", "-u", remote, branch])
            .await?;
        Ok(())
    }

    async fn fetch(&self, path: &Path, remote: &str) -> Result<(), SympheoError> {
        let _ = self.run_git(path, &["fetch", remote]).await?;
        Ok(())
    }

    async fn merge(
        &self,
        path: &Path,
        branch: &str,
        strategy: MergeStrategy,
    ) -> Result<(), SympheoError> {
        let strategy_flag = match strategy {
            MergeStrategy::Ours => "--strategy-option=ours",
            MergeStrategy::Theirs => "--strategy-option=theirs",
            MergeStrategy::Default => "",
        };
        if strategy_flag.is_empty() {
            let _ = self.run_git(path, &["merge", branch]).await?;
        } else {
            let _ = self.run_git(path, &["merge", strategy_flag, branch]).await?;
        }
        Ok(())
    }

    async fn status(&self, path: &Path) -> Result<GitStatus, SympheoError> {
        // Check for detached HEAD
        let (branch_out, _) = self.run_git(path, &["rev-parse", "--abbrev-ref", "HEAD"]).await?;
        let branch = branch_out.trim();
        let is_detached = branch == "HEAD";

        // Check for dirty working tree
        let (status_out, _) = self
            .run_git(path, &["status", "--porcelain=v1"])
            .await?;
        let lines: Vec<String> = status_out
            .lines()
            .map(|l| l.to_string())
            .filter(|l| !l.is_empty())
            .collect();

        if lines.is_empty() && !is_detached {
            Ok(GitStatus::Clean)
        } else if is_detached {
            Ok(GitStatus::DetachedHead)
        } else {
            Ok(GitStatus::Dirty(lines))
        }
    }

    async fn log(&self, path: &Path, n: usize) -> Result<Vec<CommitInfo>, SympheoError> {
        let format = "%H|%s|%an|%aI";
        let (out, _) = self
            .run_git(path, &["log", &format!("-{n}"), &format!("--format={format}")])
            .await?;
        let mut commits = vec![];
        for line in out.lines() {
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() == 4 {
                commits.push(CommitInfo {
                    hash: parts[0].to_string(),
                    message: parts[1].to_string(),
                    author: parts[2].to_string(),
                    timestamp: parts[3].to_string(),
                });
            }
        }
        Ok(commits)
    }

    async fn reset_hard(&self, path: &Path, ref_name: &str) -> Result<(), SympheoError> {
        let _ = self.run_git(path, &["reset", "--hard", ref_name]).await?;
        Ok(())
    }
}

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Default, Clone)]
    pub struct MockGitAdapter {
        pub clone_results: Arc<Mutex<Vec<Result<(), SympheoError>>>>,
        pub checkout_results: Arc<Mutex<Vec<Result<(), SympheoError>>>>,
        pub commit_results: Arc<Mutex<Vec<Result<String, SympheoError>>>>,
        pub push_results: Arc<Mutex<Vec<Result<(), SympheoError>>>>,
        pub fetch_results: Arc<Mutex<Vec<Result<(), SympheoError>>>>,
        pub merge_results: Arc<Mutex<Vec<Result<(), SympheoError>>>>,
        pub status_results: Arc<Mutex<Vec<Result<GitStatus, SympheoError>>>>,
        pub log_results: Arc<Mutex<Vec<Result<Vec<CommitInfo>, SympheoError>>>>,
        pub reset_hard_results: Arc<Mutex<Vec<Result<(), SympheoError>>>>,
    }

    #[async_trait]
    impl GitAdapter for MockGitAdapter {
        async fn clone(&self, _url: &str, _path: &Path) -> Result<(), SympheoError> {
            self.clone_results.lock().unwrap().remove(0)
        }
        async fn checkout_branch(&self, _path: &Path, _branch: &str, _create: bool) -> Result<(), SympheoError> {
            self.checkout_results.lock().unwrap().remove(0)
        }
        async fn commit(&self, _path: &Path, _message: &str, _files: &[&str]) -> Result<String, SympheoError> {
            self.commit_results.lock().unwrap().remove(0)
        }
        async fn push(&self, _path: &Path, _remote: &str, _branch: &str) -> Result<(), SympheoError> {
            self.push_results.lock().unwrap().remove(0)
        }
        async fn fetch(&self, _path: &Path, _remote: &str) -> Result<(), SympheoError> {
            self.fetch_results.lock().unwrap().remove(0)
        }
        async fn merge(&self, _path: &Path, _branch: &str, _strategy: MergeStrategy) -> Result<(), SympheoError> {
            self.merge_results.lock().unwrap().remove(0)
        }
        async fn status(&self, _path: &Path) -> Result<GitStatus, SympheoError> {
            self.status_results.lock().unwrap().remove(0)
        }
        async fn log(&self, _path: &Path, _n: usize) -> Result<Vec<CommitInfo>, SympheoError> {
            self.log_results.lock().unwrap().remove(0)
        }
        async fn reset_hard(&self, _path: &Path, _ref_name: &str) -> Result<(), SympheoError> {
            self.reset_hard_results.lock().unwrap().remove(0)
        }
    }
}
