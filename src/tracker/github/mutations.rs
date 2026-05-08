use crate::error::SympheoError;
use crate::tracker::github::GithubTracker;
use crate::tracker::model::Issue;
use serde_json::json;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

impl GithubTracker {
    pub async fn move_issue_state(
        &self,
        issue: &Issue,
        new_state: &str,
    ) -> Result<(), SympheoError> {
        let project_id = self.ensure_project_id().await?;
        let item_id = issue.project_item_id.as_ref().ok_or_else(|| {
            SympheoError::InvalidConfiguration(
                "issue.project_item_id is required to move state".into(),
            )
        })?;

        let status_field_name = "Status";
        let (field_id, option_id) = self
            .resolve_status_option(&project_id, status_field_name, new_state)
            .await?;

        let mutation = r#"
            mutation MoveProjectItem($projectId: ID!, $itemId: ID!, $fieldId: ID!, $optionId: String!) {
                updateProjectV2ItemFieldValue(input: {
                    projectId: $projectId,
                    itemId: $itemId,
                    fieldId: $fieldId,
                    value: { singleSelectOptionId: $optionId }
                }) {
                    projectV2Item { id }
                }
            }
        "#;

        let variables = json!({
            "projectId": project_id,
            "itemId": item_id,
            "fieldId": field_id,
            "optionId": option_id,
        });

        self.graphql_mutation(mutation, variables).await?;
        Ok(())
    }

    pub async fn add_comment(&self, issue: &Issue, body: &str) -> Result<(), SympheoError> {
        let subject_id = issue.node_id.as_ref().ok_or_else(|| {
            SympheoError::InvalidConfiguration("issue.node_id is required to add comment".into())
        })?;

        let mutation = r#"
            mutation AddComment($subjectId: ID!, $body: String!) {
                addComment(input: {subjectId: $subjectId, body: $body}) {
                    commentEdge { node { id } }
                }
            }
        "#;

        let variables = json!({
            "subjectId": subject_id,
            "body": body,
        });

        self.graphql_mutation(mutation, variables).await?;
        Ok(())
    }

    pub async fn update_issue_body(&self, issue: &Issue, body: &str) -> Result<(), SympheoError> {
        let issue_id = issue.node_id.as_ref().ok_or_else(|| {
            SympheoError::InvalidConfiguration(
                "issue.node_id is required to update issue body".into(),
            )
        })?;

        let mutation = r#"
            mutation UpdateIssue($id: ID!, $body: String!) {
                updateIssue(input: {id: $id, body: $body}) {
                    issue { id }
                }
            }
        "#;

        let variables = json!({
            "id": issue_id,
            "body": body,
        });

        self.graphql_mutation(mutation, variables).await?;
        Ok(())
    }

    async fn ensure_project_id(&self) -> Result<String, SympheoError> {
        {
            let lock = self
                .project_id
                .lock()
                .map_err(|e| SympheoError::TrackerApiRequest(e.to_string()))?;
            if let Some(ref id) = *lock {
                return Ok(id.clone());
            }
        }

        let query = r#"
            query($owner: String!, $projectNumber: Int!) {
                organization(login: $owner) {
                    projectV2(number: $projectNumber) { id }
                }
                user(login: $owner) {
                    projectV2(number: $projectNumber) { id }
                }
            }
        "#;

        let variables = json!({
            "owner": self.owner,
            "projectNumber": self.project_number,
        });

        let data = self.graphql_query(query, variables).await?;
        let project_id = data
            .get("organization")
            .and_then(|o| o.get("projectV2"))
            .and_then(|p| p.get("id"))
            .or_else(|| {
                data.get("user")
                    .and_then(|u| u.get("projectV2"))
                    .and_then(|p| p.get("id"))
            })
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                SympheoError::InvalidConfiguration("could not resolve project id".into())
            })?
            .to_string();

        {
            let mut lock = self
                .project_id
                .lock()
                .map_err(|e| SympheoError::TrackerApiRequest(e.to_string()))?;
            *lock = Some(project_id.clone());
        }
        Ok(project_id)
    }

    async fn resolve_status_option(
        &self,
        project_id: &str,
        field_name: &str,
        option_name: &str,
    ) -> Result<(String, String), SympheoError> {
        let option_name_lc = option_name.to_lowercase();
        {
            let cache = self
                .field_cache
                .lock()
                .map_err(|e| SympheoError::TrackerApiRequest(e.to_string()))?;
            if let Some((field_id, options)) = cache.get(field_name)
                && let Some(option_id) = options.get(&option_name_lc)
            {
                return Ok((field_id.clone(), option_id.clone()));
            }
        }

        let query = r#"
            query($projectId: ID!) {
                node(id: $projectId) {
                    ... on ProjectV2 {
                        fields(first: 20) {
                            nodes {
                                ... on ProjectV2SingleSelectField {
                                    id
                                    name
                                    options {
                                        id
                                        name
                                    }
                                }
                            }
                        }
                    }
                }
            }
        "#;

        let variables = json!({ "projectId": project_id });
        let data = self.graphql_query(query, variables).await?;

        let fields = data
            .get("node")
            .and_then(|n| n.get("fields"))
            .and_then(|f| f.get("nodes"))
            .and_then(|n| n.as_array())
            .ok_or_else(|| {
                SympheoError::InvalidConfiguration("could not fetch project fields".into())
            })?;

        for field in fields {
            let name = field.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if name.to_lowercase() == field_name.to_lowercase() {
                let field_id = field
                    .get("id")
                    .and_then(|i| i.as_str())
                    .ok_or_else(|| SympheoError::InvalidConfiguration("field missing id".into()))?
                    .to_string();

                let options_arr = field
                    .get("options")
                    .and_then(|o| o.as_array())
                    .unwrap_or(&vec![])
                    .clone();

                let mut options_map = std::collections::HashMap::new();
                for opt in &options_arr {
                    let opt_name = opt.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let opt_id = opt.get("id").and_then(|i| i.as_str()).unwrap_or("");
                    options_map.insert(opt_name.to_lowercase(), opt_id.to_string());
                }

                let option_id = options_map
                    .get(&option_name_lc)
                    .ok_or_else(|| {
                        SympheoError::InvalidConfiguration(format!(
                            "status option '{}' not found",
                            option_name
                        ))
                    })?
                    .clone();

                {
                    let mut cache = self
                        .field_cache
                        .lock()
                        .map_err(|e| SympheoError::TrackerApiRequest(e.to_string()))?;
                    cache.insert(field_name.to_string(), (field_id.clone(), options_map));
                }
                return Ok((field_id, option_id));
            }
        }

        Err(SympheoError::InvalidConfiguration(format!(
            "status field '{}' not found in project",
            field_name
        )))
    }

    async fn graphql_mutation(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<serde_json::Value, SympheoError> {
        let mut retries = 0;
        let max_retries = 3;

        loop {
            let body = json!({ "query": query, "variables": variables });
            let resp = self
                .client
                .post(format!("{}/graphql", self.endpoint))
                .json(&body)
                .send()
                .await
                .map_err(|e| SympheoError::TrackerApiRequest(e.to_string()))?;

            let status = resp.status();
            let headers = resp.headers().clone();
            let json: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| SympheoError::TrackerMalformedPayload(e.to_string()))?;

            if let Some(errors) = json.get("errors") {
                return Err(SympheoError::TrackerApiStatus(format!(
                    "GraphQL errors: {}",
                    errors
                )));
            }

            if status.is_success() {
                return Ok(json["data"].clone());
            }

            // Rate limit handling
            if let Some(remaining) = headers.get("x-ratelimit-remaining")
                && let Ok(rem_str) = remaining.to_str()
                && let Ok(rem) = rem_str.parse::<i64>()
                && rem <= 0
                && let Some(reset) = headers.get("x-ratelimit-reset")
                && let Ok(reset_str) = reset.to_str()
                && let Ok(reset_ts) = reset_str.parse::<u64>()
            {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                if reset_ts > now {
                    let wait = Duration::from_secs(reset_ts - now + 1);
                    tracing::warn!("GitHub rate limit hit, waiting {:?}", wait);
                    tokio::time::sleep(wait).await;
                    continue;
                }
            }

            // Exponential backoff on server errors
            if status.as_u16() >= 500 && retries < max_retries {
                retries += 1;
                let delay = Duration::from_millis(500 * 2_u64.pow(retries));
                tracing::warn!(
                    "GitHub API returned {}, retrying in {:?} (attempt {}/{})",
                    status,
                    delay,
                    retries,
                    max_retries
                );
                tokio::time::sleep(delay).await;
                continue;
            }

            return Err(SympheoError::TrackerApiStatus(format!(
                "HTTP {}: {}",
                status, json
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::typed::ServiceConfig;
    use crate::tracker::github::GithubTracker;
    use crate::tracker::model::Issue;
    use std::path::PathBuf;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

    fn make_config(endpoint: &str) -> ServiceConfig {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert("kind".into(), serde_json::Value::String("github".into()));
        tracker.insert(
            "api_key".into(),
            serde_json::Value::String("test-key".into()),
        );
        tracker.insert(
            "project_slug".into(),
            serde_json::Value::String("owner/repo".into()),
        );
        tracker.insert("project_number".into(), serde_json::Value::Number(1.into()));
        tracker.insert(
            "endpoint".into(),
            serde_json::Value::String(endpoint.into()),
        );
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into())
    }

    fn project_id_response() -> serde_json::Value {
        serde_json::json!({
            "data": {
                "organization": {
                    "projectV2": { "id": "proj-123" }
                }
            }
        })
    }

    fn project_fields_response() -> serde_json::Value {
        serde_json::json!({
            "data": {
                "node": {
                    "fields": {
                        "nodes": [
                            {
                                "id": "field-1",
                                "name": "Status",
                                "options": [
                                    { "id": "opt-1", "name": "Todo" },
                                    { "id": "opt-2", "name": "In Progress" },
                                    { "id": "opt-3", "name": "Done" }
                                ]
                            }
                        ]
                    }
                }
            }
        })
    }

    fn mutation_success_response() -> serde_json::Value {
        serde_json::json!({
            "data": {
                "updateProjectV2ItemFieldValue": {
                    "projectV2Item": { "id": "item-123" }
                }
            }
        })
    }

    fn comment_success_response() -> serde_json::Value {
        serde_json::json!({
            "data": {
                "addComment": {
                    "commentEdge": { "node": { "id": "comment-123" } }
                }
            }
        })
    }

    fn update_body_success_response() -> serde_json::Value {
        serde_json::json!({
            "data": {
                "updateIssue": {
                    "issue": { "id": "issue-123" }
                }
            }
        })
    }

    #[tokio::test]
    async fn test_move_issue_state_success() {
        let mock_server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(project_id_response()))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(project_fields_response()))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mutation_success_response()))
            .mount(&mock_server)
            .await;

        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let issue = Issue {
            project_item_id: Some("item-123".into()),
            ..Default::default()
        };
        assert!(tracker.move_issue_state(&issue, "Done").await.is_ok());
    }

    #[tokio::test]
    async fn test_move_issue_state_missing_project_item_id() {
        let mock_server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(project_id_response()))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let issue = Issue::default();
        let result = tracker.move_issue_state(&issue, "Done").await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SympheoError::InvalidConfiguration(_)
        ));
    }

    #[tokio::test]
    async fn test_add_comment_success() {
        let mock_server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(comment_success_response()))
            .mount(&mock_server)
            .await;

        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let issue = Issue {
            node_id: Some("issue-123".into()),
            ..Default::default()
        };
        assert!(tracker.add_comment(&issue, "Great work!").await.is_ok());
    }

    #[tokio::test]
    async fn test_add_comment_missing_node_id() {
        let mock_server = MockServer::start().await;
        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let issue = Issue::default();
        let result = tracker.add_comment(&issue, "test").await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SympheoError::InvalidConfiguration(_)
        ));
    }

    #[tokio::test]
    async fn test_update_issue_body_success() {
        let mock_server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(update_body_success_response()))
            .mount(&mock_server)
            .await;

        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let issue = Issue {
            node_id: Some("issue-123".into()),
            ..Default::default()
        };
        assert!(
            tracker
                .update_issue_body(&issue, "Updated body")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_update_issue_body_missing_node_id() {
        let mock_server = MockServer::start().await;
        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let issue = Issue::default();
        let result = tracker.update_issue_body(&issue, "test").await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SympheoError::InvalidConfiguration(_)
        ));
    }

    #[tokio::test]
    async fn test_ensure_project_id_org_path() {
        let mock_server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(project_id_response()))
            .mount(&mock_server)
            .await;

        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let id = tracker.ensure_project_id().await.unwrap();
        assert_eq!(id, "proj-123");
    }

    #[tokio::test]
    async fn test_ensure_project_id_user_path() {
        let mock_server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {
                    "user": {
                        "projectV2": { "id": "proj-user-123" }
                    }
                }
            })))
            .mount(&mock_server)
            .await;

        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let id = tracker.ensure_project_id().await.unwrap();
        assert_eq!(id, "proj-user-123");
    }

    #[tokio::test]
    async fn test_ensure_project_id_caches() {
        let mock_server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(project_id_response()))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let id1 = tracker.ensure_project_id().await.unwrap();
        let id2 = tracker.ensure_project_id().await.unwrap();
        assert_eq!(id1, id2);
    }

    #[tokio::test]
    async fn test_resolve_status_option_cache_miss() {
        let mock_server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(project_fields_response()))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let (field_id, option_id) = tracker
            .resolve_status_option("proj-123", "Status", "Done")
            .await
            .unwrap();
        assert_eq!(field_id, "field-1");
        assert_eq!(option_id, "opt-3");
    }

    #[tokio::test]
    async fn test_resolve_status_option_cache_hit() {
        let mock_server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(project_fields_response()))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let _ = tracker
            .resolve_status_option("proj-123", "Status", "Done")
            .await
            .unwrap();
        let (field_id, option_id) = tracker
            .resolve_status_option("proj-123", "Status", "Done")
            .await
            .unwrap();
        assert_eq!(field_id, "field-1");
        assert_eq!(option_id, "opt-3");
    }

    #[tokio::test]
    async fn test_resolve_status_option_field_not_found() {
        let mock_server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {
                    "node": {
                        "fields": { "nodes": [] }
                    }
                }
            })))
            .mount(&mock_server)
            .await;

        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let result = tracker
            .resolve_status_option("proj-123", "Status", "Done")
            .await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SympheoError::InvalidConfiguration(_)
        ));
    }

    #[tokio::test]
    async fn test_resolve_status_option_option_not_found() {
        let mock_server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(project_fields_response()))
            .mount(&mock_server)
            .await;

        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let result = tracker
            .resolve_status_option("proj-123", "Status", "NonExistent")
            .await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SympheoError::InvalidConfiguration(_)
        ));
    }

    #[tokio::test]
    async fn test_graphql_mutation_rate_limit_wait() {
        let mock_server = MockServer::start().await;
        let reset_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 2;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("x-ratelimit-remaining", "0")
                    .insert_header("x-ratelimit-reset", reset_ts.to_string())
                    .set_body_json(serde_json::json!({"data": null})),
            )
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mutation_success_response()))
            .mount(&mock_server)
            .await;

        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let result = tracker
            .graphql_mutation("mutation {}", serde_json::json!({}))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_graphql_mutation_500_retry_then_success() {
        let mock_server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(
                ResponseTemplate::new(500).set_body_json(serde_json::json!({"error": "boom"})),
            )
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mutation_success_response()))
            .mount(&mock_server)
            .await;

        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let result = tracker
            .graphql_mutation("mutation {}", serde_json::json!({}))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_graphql_mutation_500_exhausted() {
        let mock_server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(
                ResponseTemplate::new(500).set_body_json(serde_json::json!({"error": "boom"})),
            )
            .mount(&mock_server)
            .await;

        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let result = tracker
            .graphql_mutation("mutation {}", serde_json::json!({}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_graphql_mutation_graphql_errors() {
        let mock_server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "errors": [{"message": "Something went wrong"}]
            })))
            .mount(&mock_server)
            .await;

        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let result = tracker
            .graphql_mutation("mutation {}", serde_json::json!({}))
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("GraphQL errors"));
    }

    #[tokio::test]
    async fn test_ensure_project_id_not_found() {
        let mock_server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {
                    "organization": null,
                    "user": null
                }
            })))
            .mount(&mock_server)
            .await;

        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let result = tracker.ensure_project_id().await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SympheoError::InvalidConfiguration(_)
        ));
    }

    #[tokio::test]
    async fn test_resolve_status_option_cannot_fetch_fields() {
        let mock_server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {
                    "node": {
                        "fields": null
                    }
                }
            })))
            .mount(&mock_server)
            .await;

        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let result = tracker
            .resolve_status_option("proj-123", "Status", "Done")
            .await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SympheoError::InvalidConfiguration(_)
        ));
    }

    #[tokio::test]
    async fn test_resolve_status_option_field_missing_id() {
        let mock_server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {
                    "node": {
                        "fields": {
                            "nodes": [
                                {
                                    "name": "Status",
                                    "options": []
                                }
                            ]
                        }
                    }
                }
            })))
            .mount(&mock_server)
            .await;

        let config = make_config(&mock_server.uri());
        let tracker = GithubTracker::new(&config).unwrap();
        let result = tracker
            .resolve_status_option("proj-123", "Status", "Done")
            .await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SympheoError::InvalidConfiguration(_)
        ));
    }
}
