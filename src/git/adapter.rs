use crate::error::SympheoError;
use async_trait::async_trait;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeStrategy {
    Ours,
    Theirs,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitStatus {
    Clean,
    Dirty(Vec<String>),
    DetachedHead,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub hash: String,
    pub message: String,
    pub author: String,
    pub timestamp: String,
}

#[async_trait]
pub trait GitAdapter: Send + Sync {
    async fn clone(&self, url: &str, path: &Path) -> Result<(), SympheoError>;
    async fn checkout_branch(&self, path: &Path, branch: &str, create: bool) -> Result<(), SympheoError>;
    async fn commit(&self, path: &Path, message: &str, files: &[&str]) -> Result<String, SympheoError>;
    async fn push(&self, path: &Path, remote: &str, branch: &str) -> Result<(), SympheoError>;
    async fn fetch(&self, path: &Path, remote: &str) -> Result<(), SympheoError>;
    async fn merge(&self, path: &Path, branch: &str, strategy: MergeStrategy) -> Result<(), SympheoError>;
    async fn status(&self, path: &Path) -> Result<GitStatus, SympheoError>;
    async fn log(&self, path: &Path, n: usize) -> Result<Vec<CommitInfo>, SympheoError>;
    async fn reset_hard(&self, path: &Path, ref_name: &str) -> Result<(), SympheoError>;
}
