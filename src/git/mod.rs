pub mod adapter;
pub mod local;

pub use adapter::{CommitInfo, GitAdapter, GitStatus, MergeStrategy};
pub use local::LocalGitAdapter;
