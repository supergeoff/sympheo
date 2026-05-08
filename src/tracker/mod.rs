pub mod github;
pub mod model;

use crate::error::SympheoError;
use async_trait::async_trait;
use model::Issue;

#[async_trait]
pub trait IssueTracker: Send + Sync {
    async fn fetch_candidate_issues(&self) -> Result<Vec<Issue>, SympheoError>;
    async fn fetch_issues_by_states(&self, states: &[String]) -> Result<Vec<Issue>, SympheoError>;
    async fn fetch_issue_states_by_ids(&self, ids: &[String]) -> Result<Vec<Issue>, SympheoError>;
}
