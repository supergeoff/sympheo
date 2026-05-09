pub mod github;
pub mod model;

use crate::error::SympheoError;
use async_trait::async_trait;
use model::Issue;

#[async_trait]
pub trait IssueTracker: Send + Sync {
    /// SPEC §11.1.1: static configuration validation (required fields, auth resolution).
    /// Does NOT perform network calls. Default impl returns Ok for backward-compat.
    fn validate(&self) -> Result<(), SympheoError> {
        Ok(())
    }

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
        Err(SympheoError::UnsupportedTrackerKind(
            "create_pull_request not implemented".into(),
        ))
    }

    async fn get_linked_prs(
        &self,
        _issue: &Issue,
    ) -> Result<Vec<crate::tracker::model::PullRequest>, SympheoError> {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::model::Issue;

    struct DefaultTracker;

    #[async_trait]
    impl IssueTracker for DefaultTracker {
        async fn fetch_candidate_issues(&self) -> Result<Vec<Issue>, SympheoError> {
            Ok(vec![])
        }
        async fn fetch_issues_by_states(
            &self,
            _states: &[String],
        ) -> Result<Vec<Issue>, SympheoError> {
            Ok(vec![])
        }
        async fn fetch_issue_states_by_ids(
            &self,
            _ids: &[String],
        ) -> Result<Vec<Issue>, SympheoError> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn test_default_move_issue_state() {
        let tracker = DefaultTracker;
        let issue = Issue::default();
        assert!(tracker.move_issue_state(&issue, "done").await.is_ok());
    }

    #[tokio::test]
    async fn test_default_add_comment() {
        let tracker = DefaultTracker;
        let issue = Issue::default();
        assert!(tracker.add_comment(&issue, "hello").await.is_ok());
    }

    #[tokio::test]
    async fn test_default_update_issue_body() {
        let tracker = DefaultTracker;
        let issue = Issue::default();
        assert!(tracker.update_issue_body(&issue, "body").await.is_ok());
    }

    #[tokio::test]
    async fn test_default_create_pull_request() {
        let tracker = DefaultTracker;
        let issue = Issue::default();
        let result = tracker
            .create_pull_request(&issue, "title", "body", "head", "base")
            .await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SympheoError::UnsupportedTrackerKind(_)
        ));
    }

    #[tokio::test]
    async fn test_default_get_linked_prs() {
        let tracker = DefaultTracker;
        let issue = Issue::default();
        let prs = tracker.get_linked_prs(&issue).await.unwrap();
        assert!(prs.is_empty());
    }
}
