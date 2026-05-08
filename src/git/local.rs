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

    async fn run_git(&self, path: &Path, args: &[&str]) -> Result<(String, String), SympheoError> {
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
        let _ = self.run_git(path, &["push", "-u", remote, branch]).await?;
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
            let _ = self
                .run_git(path, &["merge", strategy_flag, branch])
                .await?;
        }
        Ok(())
    }

    async fn status(&self, path: &Path) -> Result<GitStatus, SympheoError> {
        // Check for detached HEAD
        let (branch_out, _) = self
            .run_git(path, &["rev-parse", "--abbrev-ref", "HEAD"])
            .await?;
        let branch = branch_out.trim();
        let is_detached = branch == "HEAD";

        // Check for dirty working tree
        let (status_out, _) = self.run_git(path, &["status", "--porcelain=v1"]).await?;
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
            .run_git(
                path,
                &["log", &format!("-{n}"), &format!("--format={format}")],
            )
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

    type Results<T> = Arc<Mutex<Vec<Result<T, SympheoError>>>>;

    #[derive(Debug, Default, Clone)]
    pub struct MockGitAdapter {
        pub clone_results: Results<()>,
        pub checkout_results: Results<()>,
        pub commit_results: Results<String>,
        pub push_results: Results<()>,
        pub fetch_results: Results<()>,
        pub merge_results: Results<()>,
        pub status_results: Results<GitStatus>,
        pub log_results: Results<Vec<CommitInfo>>,
        pub reset_hard_results: Results<()>,
    }

    #[async_trait]
    impl GitAdapter for MockGitAdapter {
        async fn clone(&self, _url: &str, _path: &Path) -> Result<(), SympheoError> {
            self.clone_results.lock().unwrap().remove(0)
        }
        async fn checkout_branch(
            &self,
            _path: &Path,
            _branch: &str,
            _create: bool,
        ) -> Result<(), SympheoError> {
            self.checkout_results.lock().unwrap().remove(0)
        }
        async fn commit(
            &self,
            _path: &Path,
            _message: &str,
            _files: &[&str],
        ) -> Result<String, SympheoError> {
            self.commit_results.lock().unwrap().remove(0)
        }
        async fn push(
            &self,
            _path: &Path,
            _remote: &str,
            _branch: &str,
        ) -> Result<(), SympheoError> {
            self.push_results.lock().unwrap().remove(0)
        }
        async fn fetch(&self, _path: &Path, _remote: &str) -> Result<(), SympheoError> {
            self.fetch_results.lock().unwrap().remove(0)
        }
        async fn merge(
            &self,
            _path: &Path,
            _branch: &str,
            _strategy: MergeStrategy,
        ) -> Result<(), SympheoError> {
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

#[cfg(test)]
mod local_tests {
    use super::*;

    async fn init_repo(path: &std::path::Path) {
        let _ = tokio::process::Command::new("git")
            .args(["init", "-b", "main"])
            .arg(path)
            .output()
            .await
            .expect("git init failed");
        let _ = tokio::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["config", "user.email", "test@example.com"])
            .output()
            .await
            .expect("git config email failed");
        let _ = tokio::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["config", "user.name", "Test User"])
            .output()
            .await
            .expect("git config name failed");
    }

    async fn commit_file(path: &std::path::Path, filename: &str, content: &str, message: &str) {
        let file_path = path.join(filename);
        tokio::fs::write(&file_path, content).await.unwrap();
        let _ = tokio::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["add", filename])
            .output()
            .await
            .expect("git add failed");
        let _ = tokio::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["commit", "-m", message])
            .output()
            .await
            .expect("git commit failed");
    }

    #[tokio::test]
    async fn test_local_git_adapter_clone() {
        let adapter = LocalGitAdapter::new();
        let src = std::env::temp_dir().join(format!("sympheo_git_src_{}", std::process::id()));
        let dst = std::env::temp_dir().join(format!("sympheo_git_dst_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&src).await;
        let _ = tokio::fs::remove_dir_all(&dst).await;

        init_repo(&src).await;
        commit_file(&src, "README.md", "# hello", "initial").await;

        let result = adapter.clone(src.to_str().unwrap(), &dst).await;
        assert!(result.is_ok(), "{:?}", result);
        assert!(dst.join(".git").exists());
        assert!(dst.join("README.md").exists());

        let _ = tokio::fs::remove_dir_all(&src).await;
        let _ = tokio::fs::remove_dir_all(&dst).await;
    }

    #[tokio::test]
    async fn test_local_git_adapter_clone_failure() {
        let adapter = LocalGitAdapter::new();
        let dst = std::env::temp_dir().join(format!("sympheo_git_bad_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&dst).await;
        let result = adapter.clone("/nonexistent/repo", &dst).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_local_git_adapter_checkout_branch() {
        let adapter = LocalGitAdapter::new();
        let repo = std::env::temp_dir().join(format!("sympheo_git_co_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&repo).await;
        init_repo(&repo).await;
        commit_file(&repo, "f.txt", "a", "c1").await;

        adapter
            .checkout_branch(&repo, "feature", true)
            .await
            .unwrap();
        let (branch, _) = adapter
            .run_git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"])
            .await
            .unwrap();
        assert_eq!(branch.trim(), "feature");

        let _ = tokio::fs::remove_dir_all(&repo).await;
    }

    #[tokio::test]
    async fn test_local_git_adapter_commit() {
        let adapter = LocalGitAdapter::new();
        let repo = std::env::temp_dir().join(format!("sympheo_git_commit_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&repo).await;
        init_repo(&repo).await;
        commit_file(&repo, "a.txt", "hello", "first").await;

        tokio::fs::write(repo.join("b.txt"), "world").await.unwrap();
        let hash = adapter.commit(&repo, "add b", &["b.txt"]).await.unwrap();
        assert!(!hash.is_empty());

        let (log_out, _) = adapter
            .run_git(&repo, &["log", "-1", "--format=%s"])
            .await
            .unwrap();
        assert_eq!(log_out.trim(), "add b");

        let _ = tokio::fs::remove_dir_all(&repo).await;
    }

    #[tokio::test]
    async fn test_local_git_adapter_status_clean() {
        let adapter = LocalGitAdapter::new();
        let repo = std::env::temp_dir().join(format!("sympheo_git_clean_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&repo).await;
        init_repo(&repo).await;
        commit_file(&repo, "a.txt", "hello", "first").await;

        let status = adapter.status(&repo).await.unwrap();
        assert!(matches!(status, GitStatus::Clean));

        let _ = tokio::fs::remove_dir_all(&repo).await;
    }

    #[tokio::test]
    async fn test_local_git_adapter_status_dirty() {
        let adapter = LocalGitAdapter::new();
        let repo = std::env::temp_dir().join(format!("sympheo_git_dirty_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&repo).await;
        init_repo(&repo).await;
        commit_file(&repo, "a.txt", "hello", "first").await;

        tokio::fs::write(repo.join("b.txt"), "world").await.unwrap();
        let status = adapter.status(&repo).await.unwrap();
        assert!(matches!(status, GitStatus::Dirty(_)));

        let _ = tokio::fs::remove_dir_all(&repo).await;
    }

    #[tokio::test]
    async fn test_local_git_adapter_status_detached() {
        let adapter = LocalGitAdapter::new();
        let repo =
            std::env::temp_dir().join(format!("sympheo_git_detached_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&repo).await;
        init_repo(&repo).await;
        commit_file(&repo, "a.txt", "hello", "first").await;

        let (hash, _) = adapter
            .run_git(&repo, &["rev-parse", "HEAD"])
            .await
            .unwrap();
        adapter
            .checkout_branch(&repo, hash.trim(), false)
            .await
            .unwrap();

        let status = adapter.status(&repo).await.unwrap();
        assert!(matches!(status, GitStatus::DetachedHead));

        let _ = tokio::fs::remove_dir_all(&repo).await;
    }

    #[tokio::test]
    async fn test_local_git_adapter_log() {
        let adapter = LocalGitAdapter::new();
        let repo = std::env::temp_dir().join(format!("sympheo_git_log_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&repo).await;
        init_repo(&repo).await;
        commit_file(&repo, "a.txt", "hello", "first").await;
        commit_file(&repo, "b.txt", "world", "second").await;

        let commits = adapter.log(&repo, 2).await.unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].message, "second");
        assert_eq!(commits[1].message, "first");

        let _ = tokio::fs::remove_dir_all(&repo).await;
    }

    #[tokio::test]
    async fn test_local_git_adapter_reset_hard() {
        let adapter = LocalGitAdapter::new();
        let repo = std::env::temp_dir().join(format!("sympheo_git_reset_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&repo).await;
        init_repo(&repo).await;
        commit_file(&repo, "a.txt", "hello", "first").await;
        commit_file(&repo, "b.txt", "world", "second").await;

        let (first_hash, _) = adapter
            .run_git(&repo, &["rev-parse", "HEAD~1"])
            .await
            .unwrap();
        adapter.reset_hard(&repo, first_hash.trim()).await.unwrap();

        let status = adapter.status(&repo).await.unwrap();
        assert!(matches!(status, GitStatus::Clean));
        assert!(!repo.join("b.txt").exists());

        let _ = tokio::fs::remove_dir_all(&repo).await;
    }

    #[tokio::test]
    async fn test_local_git_adapter_merge_default() {
        let adapter = LocalGitAdapter::new();
        let repo = std::env::temp_dir().join(format!("sympheo_git_merge_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&repo).await;
        init_repo(&repo).await;
        commit_file(&repo, "a.txt", "hello", "first").await;

        adapter
            .checkout_branch(&repo, "feature", true)
            .await
            .unwrap();
        commit_file(&repo, "b.txt", "world", "on feature").await;
        adapter.checkout_branch(&repo, "main", false).await.unwrap();
        adapter
            .merge(&repo, "feature", MergeStrategy::Default)
            .await
            .unwrap();

        let status = adapter.status(&repo).await.unwrap();
        assert!(matches!(status, GitStatus::Clean));

        let _ = tokio::fs::remove_dir_all(&repo).await;
    }

    #[tokio::test]
    async fn test_local_git_adapter_fetch() {
        let adapter = LocalGitAdapter::new();
        let remote =
            std::env::temp_dir().join(format!("sympheo_git_remote_{}", std::process::id()));
        let local = std::env::temp_dir().join(format!("sympheo_git_local_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&remote).await;
        let _ = tokio::fs::remove_dir_all(&local).await;

        init_repo(&remote).await;
        commit_file(&remote, "a.txt", "hello", "first").await;

        init_repo(&local).await;
        let _ = tokio::process::Command::new("git")
            .arg("-C")
            .arg(&local)
            .args(["remote", "add", "origin", remote.to_str().unwrap()])
            .output()
            .await
            .unwrap();

        adapter.fetch(&local, "origin").await.unwrap();

        let _ = tokio::fs::remove_dir_all(&remote).await;
        let _ = tokio::fs::remove_dir_all(&local).await;
    }

    #[tokio::test]
    async fn test_local_git_adapter_run_git_failure() {
        let adapter = LocalGitAdapter::new();
        let repo = std::env::temp_dir().join(format!("sympheo_git_bad_cmd_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&repo).await;
        tokio::fs::create_dir_all(&repo).await.unwrap();
        let result = adapter.run_git(&repo, &["status"]).await;
        assert!(result.is_err());
        let _ = tokio::fs::remove_dir_all(&repo).await;
    }

    #[tokio::test]
    async fn test_local_git_adapter_push_failure() {
        let adapter = LocalGitAdapter::new();
        let repo =
            std::env::temp_dir().join(format!("sympheo_git_push_bad_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&repo).await;
        init_repo(&repo).await;
        let result = adapter.push(&repo, "origin", "main").await;
        assert!(result.is_err());
        let _ = tokio::fs::remove_dir_all(&repo).await;
    }

    #[tokio::test]
    async fn test_local_git_adapter_merge_failure() {
        let adapter = LocalGitAdapter::new();
        let repo =
            std::env::temp_dir().join(format!("sympheo_git_merge_bad_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&repo).await;
        init_repo(&repo).await;
        let result = adapter
            .merge(&repo, "nonexistent", MergeStrategy::Default)
            .await;
        assert!(result.is_err());
        let _ = tokio::fs::remove_dir_all(&repo).await;
    }

    #[tokio::test]
    async fn test_local_git_adapter_clone_no_parent() {
        let adapter = LocalGitAdapter::new();
        let result = adapter.clone("url", std::path::Path::new("/")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_local_git_adapter_clone_no_file_name() {
        let adapter = LocalGitAdapter::new();
        let tmp = std::env::temp_dir();
        let result = adapter.clone("url", &tmp).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_local_git_adapter_merge_ours() {
        let adapter = LocalGitAdapter::new();
        let repo =
            std::env::temp_dir().join(format!("sympheo_git_merge_ours_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&repo).await;
        init_repo(&repo).await;
        commit_file(&repo, "a.txt", "hello", "first").await;

        adapter
            .checkout_branch(&repo, "feature", true)
            .await
            .unwrap();
        commit_file(&repo, "a.txt", "world", "on feature").await;
        adapter.checkout_branch(&repo, "main", false).await.unwrap();

        // main also modifies a.txt to create conflict
        tokio::fs::write(repo.join("a.txt"), "main text")
            .await
            .unwrap();
        let _ = tokio::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["add", "a.txt"])
            .output()
            .await
            .unwrap();
        let _ = tokio::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["commit", "-m", "main change"])
            .output()
            .await
            .unwrap();

        adapter
            .merge(&repo, "feature", MergeStrategy::Ours)
            .await
            .unwrap();

        let status = adapter.status(&repo).await.unwrap();
        assert!(matches!(status, GitStatus::Clean));

        let _ = tokio::fs::remove_dir_all(&repo).await;
    }

    #[test]
    fn test_local_git_adapter_default() {
        let adapter = LocalGitAdapter;
        let _ = adapter;
    }

    #[tokio::test]
    async fn test_local_git_adapter_push_success() {
        let adapter = LocalGitAdapter::new();
        let remote =
            std::env::temp_dir().join(format!("sympheo_git_push_remote_{}", std::process::id()));
        let local =
            std::env::temp_dir().join(format!("sympheo_git_push_local_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&remote).await;
        let _ = tokio::fs::remove_dir_all(&local).await;

        // Init bare remote
        let _ = tokio::process::Command::new("git")
            .args(["init", "--bare", "-b", "main"])
            .arg(&remote)
            .output()
            .await
            .unwrap();

        init_repo(&local).await;
        tokio::fs::write(local.join("a.txt"), "hello")
            .await
            .unwrap();
        let _ = tokio::process::Command::new("git")
            .arg("-C")
            .arg(&local)
            .args(["add", "a.txt"])
            .output()
            .await
            .unwrap();
        let _ = tokio::process::Command::new("git")
            .arg("-C")
            .arg(&local)
            .args(["commit", "-m", "first"])
            .output()
            .await
            .unwrap();
        let _ = tokio::process::Command::new("git")
            .arg("-C")
            .arg(&local)
            .args(["remote", "add", "origin", remote.to_str().unwrap()])
            .output()
            .await
            .unwrap();

        adapter.push(&local, "origin", "main").await.unwrap();

        let _ = tokio::fs::remove_dir_all(&remote).await;
        let _ = tokio::fs::remove_dir_all(&local).await;
    }

    #[tokio::test]
    async fn test_local_git_adapter_merge_theirs() {
        let adapter = LocalGitAdapter::new();
        let repo =
            std::env::temp_dir().join(format!("sympheo_git_merge_theirs_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&repo).await;
        init_repo(&repo).await;
        commit_file(&repo, "a.txt", "hello", "first").await;

        adapter
            .checkout_branch(&repo, "feature", true)
            .await
            .unwrap();
        commit_file(&repo, "a.txt", "world", "on feature").await;
        adapter.checkout_branch(&repo, "main", false).await.unwrap();

        tokio::fs::write(repo.join("a.txt"), "main text")
            .await
            .unwrap();
        let _ = tokio::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["add", "a.txt"])
            .output()
            .await
            .unwrap();
        let _ = tokio::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["commit", "-m", "main change"])
            .output()
            .await
            .unwrap();

        adapter
            .merge(&repo, "feature", MergeStrategy::Theirs)
            .await
            .unwrap();

        let status = adapter.status(&repo).await.unwrap();
        assert!(matches!(status, GitStatus::Clean));

        let _ = tokio::fs::remove_dir_all(&repo).await;
    }
}
