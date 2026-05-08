use crate::config::typed::ServiceConfig;
use crate::error::SympheoError;
use crate::tracker::IssueTracker;
use crate::tracker::model::Issue;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;

pub mod mutations;

pub(crate) type FieldCache = HashMap<String, (String, HashMap<String, String>)>;

pub struct GithubTracker {
    client: reqwest::Client,
    owner: String,
    repo: String,
    project_number: i64,
    endpoint: String,
    project_id: Mutex<Option<String>>,
    field_cache: Mutex<FieldCache>,
    fetch_blocked_by: bool,
}

impl GithubTracker {
    pub fn new(config: &ServiceConfig) -> Result<Self, SympheoError> {
        let api_key = config
            .tracker_api_key()
            .ok_or(SympheoError::MissingTrackerApiKey)?;
        let project = config
            .tracker_project_slug()
            .ok_or(SympheoError::MissingTrackerProjectSlug)?;
        let project_number = config.tracker_project_number().ok_or_else(|| {
            SympheoError::InvalidConfiguration(
                "tracker.project_number is required for github projects".into(),
            )
        })?;
        let parts: Vec<&str> = project.split('/').collect();
        if parts.len() != 2 {
            return Err(SympheoError::InvalidConfiguration(
                "tracker.project_slug must be owner/repo".into(),
            ));
        }
        let mut headers = HeaderMap::new();
        let auth = HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|e| SympheoError::InvalidConfiguration(e.to_string()))?;
        headers.insert(AUTHORIZATION, auth);
        headers.insert(USER_AGENT, HeaderValue::from_static("sympheo/0.1.0"));
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| SympheoError::TrackerApiRequest(e.to_string()))?;
        Ok(Self {
            client,
            owner: parts[0].to_string(),
            repo: parts[1].to_string(),
            project_number,
            endpoint: config.tracker_endpoint(),
            project_id: Mutex::new(None),
            field_cache: Mutex::new(HashMap::new()),
            fetch_blocked_by: config.fetch_blocked_by(),
        })
    }

    async fn graphql_query(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<serde_json::Value, SympheoError> {
        let body = json!({ "query": query, "variables": variables });
        let resp = self
            .client
            .post(format!("{}/graphql", self.endpoint))
            .json(&body)
            .send()
            .await
            .map_err(|e| SympheoError::TrackerApiRequest(e.to_string()))?;
        let status = resp.status();
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
        if !status.is_success() {
            return Err(SympheoError::TrackerApiStatus(format!(
                "HTTP {}: {}",
                status, json
            )));
        }
        Ok(json["data"].clone())
    }

    async fn fetch_project_items(&self) -> Result<Vec<serde_json::Value>, SympheoError> {
        // Determine if owner is an organization or a user
        let org_query = r#"
            query($owner: String!, $projectNumber: Int!) {
              organization(login: $owner) {
                projectV2(number: $projectNumber) { id }
              }
            }
        "#;
        let is_org = match self
            .graphql_query(
                org_query,
                json!({"owner": &self.owner, "projectNumber": self.project_number}),
            )
            .await
        {
            Ok(org_data) => org_data
                .get("organization")
                .and_then(|o| o.get("projectV2"))
                .is_some(),
            Err(_) => false, // Not an org (or project doesn't exist on org), try user
        };

        let query_template = if is_org {
            r#"
            query($owner: String!, $projectNumber: Int!, $first: Int!, $after: String) {
              organization(login: $owner) {
                projectV2(number: $projectNumber) {
                  items(first: $first, after: $after) {
                    nodes {
                      id
                      content {
                        ... on Issue {
                          id
                          number
                          title
                          body
                          state
                          labels(first: 10) { nodes { name } }
                          createdAt
                          updatedAt
                          url
                          repository { name owner { login } }
                        }
                      }
                      fieldValues(first: 20) {
                        nodes {
                          ... on ProjectV2ItemFieldSingleSelectValue {
                            field { ... on ProjectV2FieldCommon { name } }
                            name
                          }
                        }
                      }
                      linkedItems(first: 20) {
                        nodes {
                          ... on Issue {
                            id
                            number
                            state
                          }
                        }
                      }
                    }
                    pageInfo { hasNextPage endCursor }
                  }
                }
              }
            }
            "#
        } else {
            r#"
            query($owner: String!, $projectNumber: Int!, $first: Int!, $after: String) {
              user(login: $owner) {
                projectV2(number: $projectNumber) {
                  items(first: $first, after: $after) {
                    nodes {
                      id
                      content {
                        ... on Issue {
                          id
                          number
                          title
                          body
                          state
                          labels(first: 10) { nodes { name } }
                          createdAt
                          updatedAt
                          url
                          repository { name owner { login } }
                        }
                      }
                      fieldValues(first: 20) {
                        nodes {
                          ... on ProjectV2ItemFieldSingleSelectValue {
                            field { ... on ProjectV2FieldCommon { name } }
                            name
                          }
                        }
                      }
                      linkedItems(first: 20) {
                        nodes {
                          ... on Issue {
                            id
                            number
                            state
                          }
                        }
                      }
                    }
                    pageInfo { hasNextPage endCursor }
                  }
                }
              }
            }
            "#
        };

        let mut all = vec![];
        let mut cursor: Option<String> = None;
        loop {
            let vars = json!({
                "owner": self.owner,
                "projectNumber": self.project_number,
                "first": 50,
                "after": cursor,
            });
            let data = self.graphql_query(query_template, vars).await?;
            let root = if is_org {
                data.get("organization")
            } else {
                data.get("user")
            };
            let items = root
                .and_then(|r| r.get("projectV2"))
                .and_then(|p| p.get("items"))
                .and_then(|i| i.get("nodes"))
                .and_then(|n| n.as_array())
                .cloned()
                .unwrap_or_default();
            let page_info = root
                .and_then(|r| r.get("projectV2"))
                .and_then(|p| p.get("items"))
                .and_then(|i| i.get("pageInfo"));
            let has_next = page_info
                .and_then(|pi| pi.get("hasNextPage"))
                .and_then(|h| h.as_bool())
                .unwrap_or(false);
            let next_cursor = page_info
                .and_then(|pi| pi.get("endCursor"))
                .and_then(|c| c.as_str())
                .map(|s| s.to_string());

            all.extend(items);
            if !has_next || next_cursor.is_none() {
                break;
            }
            cursor = next_cursor;
        }
        Ok(all)
    }

    fn extract_status(&self, item: &serde_json::Value) -> Option<String> {
        item.get("fieldValues")
            .and_then(|fv| fv.get("nodes"))
            .and_then(|nodes| nodes.as_array())
            .and_then(|arr| {
                arr.iter().find_map(|node| {
                    let field_name = node
                        .get("field")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())?;
                    if field_name == "Status" {
                        node.get("name")
                            .and_then(|n| n.as_str())
                            .map(|s| s.to_lowercase())
                    } else {
                        None
                    }
                })
            })
    }

    fn normalize_item(&self, item: &serde_json::Value) -> Option<Issue> {
        let content = item.get("content")?;
        content.get("number")?;

        let repo_name = content
            .get("repository")
            .and_then(|r| r.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("");
        if repo_name != self.repo {
            return None;
        }

        let number = content.get("number")?.as_i64()?;
        let title = content
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        let body = content
            .get("body")
            .and_then(|b| b.as_str())
            .map(|s| s.to_string());
        let labels: Vec<String> = content
            .get("labels")
            .and_then(|l| l.get("nodes"))
            .and_then(|n| n.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|node| {
                        node.get("name")
                            .and_then(|n| n.as_str())
                            .map(|s| s.to_lowercase())
                    })
                    .collect()
            })
            .unwrap_or_default();

        let status = self.extract_status(item);
        let state = status.unwrap_or_else(|| {
            content
                .get("state")
                .and_then(|s| s.as_str())
                .unwrap_or("open")
                .to_lowercase()
        });

        let node_id = content
            .get("id")
            .and_then(|n| n.as_str())
            .map(|s| s.to_string());
        let project_item_id = item
            .get("id")
            .and_then(|n| n.as_str())
            .map(|s| s.to_string());

        let blocked_by = if self.fetch_blocked_by {
            item.get("linkedItems")
                .and_then(|l| l.get("nodes"))
                .and_then(|n| n.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|node| {
                            let id = node
                                .get("id")
                                .and_then(|n| n.as_str())
                                .map(|s| s.to_string());
                            let number = node
                                .get("number")
                                .and_then(|n| n.as_i64())
                                .map(|n| n.to_string());
                            let state = node
                                .get("state")
                                .and_then(|s| s.as_str())
                                .map(|s| s.to_lowercase());
                            crate::tracker::model::BlockerRef {
                                id,
                                identifier: number,
                                state,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            vec![]
        };

        Some(Issue {
            id: number.to_string(),
            identifier: format!("{}-{}", self.repo.to_uppercase(), number),
            title,
            description: body,
            priority: None,
            state,
            branch_name: None,
            url: content
                .get("url")
                .and_then(|u| u.as_str())
                .map(|s| s.to_string()),
            labels,
            blocked_by,
            node_id,
            project_item_id,
            created_at: content
                .get("createdAt")
                .and_then(|s| s.as_str())
                .and_then(|s| s.parse::<DateTime<Utc>>().ok()),
            updated_at: content
                .get("updatedAt")
                .and_then(|s| s.as_str())
                .and_then(|s| s.parse::<DateTime<Utc>>().ok()),
        })
    }
}

#[async_trait]
impl IssueTracker for GithubTracker {
    async fn fetch_candidate_issues(&self) -> Result<Vec<Issue>, SympheoError> {
        let items = self.fetch_project_items().await?;
        let issues: Vec<Issue> = items
            .iter()
            .filter_map(|i| self.normalize_item(i))
            .collect();
        Ok(issues)
    }

    async fn fetch_issues_by_states(&self, states: &[String]) -> Result<Vec<Issue>, SympheoError> {
        if states.is_empty() {
            return Ok(vec![]);
        }
        let items = self.fetch_project_items().await?;
        let issues: Vec<Issue> = items
            .iter()
            .filter_map(|i| self.normalize_item(i))
            .filter(|issue| states.contains(&issue.state.to_lowercase()))
            .collect();
        Ok(issues)
    }

    async fn fetch_issue_states_by_ids(&self, ids: &[String]) -> Result<Vec<Issue>, SympheoError> {
        let items = self.fetch_project_items().await?;
        let issues: Vec<Issue> = items
            .iter()
            .filter_map(|i| self.normalize_item(i))
            .filter(|issue| ids.contains(&issue.id))
            .collect();
        Ok(issues)
    }

    async fn move_issue_state(&self, issue: &Issue, new_state: &str) -> Result<(), SympheoError> {
        self.move_issue_state(issue, new_state).await
    }

    async fn add_comment(&self, issue: &Issue, body: &str) -> Result<(), SympheoError> {
        self.add_comment(issue, body).await
    }

    async fn update_issue_body(&self, issue: &Issue, body: &str) -> Result<(), SympheoError> {
        self.update_issue_body(issue, body).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_config(slug: &str, number: i64, api_key: &str) -> ServiceConfig {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert("kind".into(), serde_json::Value::String("github".into()));
        tracker.insert("api_key".into(), serde_json::Value::String(api_key.into()));
        tracker.insert(
            "project_slug".into(),
            serde_json::Value::String(slug.into()),
        );
        tracker.insert(
            "project_number".into(),
            serde_json::Value::Number(number.into()),
        );
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into())
    }

    #[test]
    fn test_github_tracker_new_ok() {
        let config = make_config("owner/repo", 1, "key");
        let tracker = GithubTracker::new(&config).unwrap();
        assert_eq!(tracker.owner, "owner");
        assert_eq!(tracker.repo, "repo");
        assert_eq!(tracker.project_number, 1);
    }

    #[test]
    fn test_github_tracker_new_invalid_slug() {
        let config = make_config("invalid", 1, "key");
        let result = GithubTracker::new(&config);
        assert!(matches!(result, Err(SympheoError::InvalidConfiguration(_))));
    }

    #[test]
    fn test_github_tracker_new_missing_api_key() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert("kind".into(), serde_json::Value::String("github".into()));
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let result = GithubTracker::new(&config);
        assert!(matches!(result, Err(SympheoError::MissingTrackerApiKey)));
    }

    #[test]
    fn test_extract_status_found() {
        let config = make_config("owner/repo", 1, "key");
        let tracker = GithubTracker::new(&config).unwrap();
        let item = serde_json::json!({
            "fieldValues": {
                "nodes": [
                    {
                        "field": { "name": "Status" },
                        "name": "In Progress"
                    },
                    {
                        "field": { "name": "Priority" },
                        "name": "High"
                    }
                ]
            }
        });
        assert_eq!(
            tracker.extract_status(&item),
            Some("in progress".to_string())
        );
    }

    #[test]
    fn test_extract_status_no_match() {
        let config = make_config("owner/repo", 1, "key");
        let tracker = GithubTracker::new(&config).unwrap();
        let item = serde_json::json!({
            "fieldValues": {
                "nodes": [
                    {
                        "field": { "name": "Priority" },
                        "name": "High"
                    }
                ]
            }
        });
        assert_eq!(tracker.extract_status(&item), None);
    }

    #[test]
    fn test_extract_status_missing_field_values() {
        let config = make_config("owner/repo", 1, "key");
        let tracker = GithubTracker::new(&config).unwrap();
        let item = serde_json::json!({});
        assert_eq!(tracker.extract_status(&item), None);
    }

    #[test]
    fn test_normalize_item_ok() {
        let config = make_config("owner/repo", 1, "key");
        let tracker = GithubTracker::new(&config).unwrap();
        let item = serde_json::json!({
            "content": {
                "number": 42,
                "title": "Fix bug",
                "body": "Description here",
                "state": "OPEN",
                "labels": { "nodes": [{ "name": "bug" }, { "name": "urgent" }] },
                "createdAt": "2024-01-15T10:00:00Z",
                "updatedAt": "2024-01-16T12:00:00Z",
                "url": "https://github.com/owner/repo/issues/42",
                "repository": { "name": "repo", "owner": { "login": "owner" } }
            },
            "fieldValues": {
                "nodes": [
                    { "field": { "name": "Status" }, "name": "In Progress" }
                ]
            }
        });
        let issue = tracker.normalize_item(&item).unwrap();
        assert_eq!(issue.id, "42");
        assert_eq!(issue.identifier, "REPO-42");
        assert_eq!(issue.title, "Fix bug");
        assert_eq!(issue.description, Some("Description here".to_string()));
        assert_eq!(issue.state, "in progress");
        assert_eq!(issue.labels, vec!["bug", "urgent"]);
        assert_eq!(
            issue.url,
            Some("https://github.com/owner/repo/issues/42".to_string())
        );
        assert!(issue.created_at.is_some());
        assert!(issue.updated_at.is_some());
    }

    #[test]
    fn test_normalize_item_wrong_repo() {
        let config = make_config("owner/repo", 1, "key");
        let tracker = GithubTracker::new(&config).unwrap();
        let item = serde_json::json!({
            "content": {
                "number": 42,
                "title": "Fix bug",
                "repository": { "name": "other-repo", "owner": { "login": "owner" } }
            }
        });
        assert!(tracker.normalize_item(&item).is_none());
    }

    #[test]
    fn test_normalize_item_missing_number() {
        let config = make_config("owner/repo", 1, "key");
        let tracker = GithubTracker::new(&config).unwrap();
        let item = serde_json::json!({
            "content": {
                "title": "Fix bug",
                "repository": { "name": "repo", "owner": { "login": "owner" } }
            }
        });
        assert!(tracker.normalize_item(&item).is_none());
    }

    #[test]
    fn test_normalize_item_no_status_field() {
        let config = make_config("owner/repo", 1, "key");
        let tracker = GithubTracker::new(&config).unwrap();
        let item = serde_json::json!({
            "content": {
                "number": 1,
                "title": "T",
                "state": "CLOSED",
                "repository": { "name": "repo", "owner": { "login": "owner" } }
            }
        });
        let issue = tracker.normalize_item(&item).unwrap();
        assert_eq!(issue.state, "closed");
    }

    #[test]
    fn test_normalize_item_no_body() {
        let config = make_config("owner/repo", 1, "key");
        let tracker = GithubTracker::new(&config).unwrap();
        let item = serde_json::json!({
            "content": {
                "number": 1,
                "title": "T",
                "state": "OPEN",
                "labels": { "nodes": [] },
                "repository": { "name": "repo", "owner": { "login": "owner" } }
            }
        });
        let issue = tracker.normalize_item(&item).unwrap();
        assert_eq!(issue.description, None);
    }

    #[test]
    fn test_normalize_item_no_labels() {
        let config = make_config("owner/repo", 1, "key");
        let tracker = GithubTracker::new(&config).unwrap();
        let item = serde_json::json!({
            "content": {
                "number": 1,
                "title": "T",
                "state": "OPEN",
                "repository": { "name": "repo", "owner": { "login": "owner" } }
            }
        });
        let issue = tracker.normalize_item(&item).unwrap();
        assert!(issue.labels.is_empty());
    }

    #[test]
    fn test_github_tracker_new_missing_project_slug() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert("kind".into(), serde_json::Value::String("github".into()));
        tracker.insert("api_key".into(), serde_json::Value::String("key".into()));
        tracker.insert("project_number".into(), serde_json::Value::Number(1.into()));
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let result = GithubTracker::new(&config);
        assert!(matches!(
            result,
            Err(SympheoError::MissingTrackerProjectSlug)
        ));
    }

    #[test]
    fn test_github_tracker_new_missing_project_number() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert("kind".into(), serde_json::Value::String("github".into()));
        tracker.insert("api_key".into(), serde_json::Value::String("key".into()));
        tracker.insert(
            "project_slug".into(),
            serde_json::Value::String("owner/repo".into()),
        );
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let result = GithubTracker::new(&config);
        assert!(matches!(result, Err(SympheoError::InvalidConfiguration(_))));
    }
}
