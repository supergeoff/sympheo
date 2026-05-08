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

    async fn move_issue_state(&self, _issue: &Issue, _new_state: &str) -> Result<(), SympheoError> {
        Ok(())
    }

    async fn add_comment(&self, _issue: &Issue, _body: &str) -> Result<(), SympheoError> {
        Ok(())
    }

    async fn update_issue_body(&self, _issue: &Issue, _body: &str) -> Result<(), SympheoError> {
        Ok(())
    }

    async fn create_pull_request(
        &self,
        _issue: &Issue,
        _title: &str,
        _body: &str,
        _head_branch: &str,
        _base_branch: &str,
    ) -> Result<crate::tracker::model::PullRequest, SympheoError> {
        Err(SympheoError::UnsupportedTrackerKind("create_pull_request not implemented".into()))
    }

    async fn get_linked_prs(&self, _issue: &Issue) -> Result<Vec<crate::tracker::model::PullRequest>, SympheoError> {
        Ok(vec![])
    }
}
