use crate::config::typed::ServiceConfig;
use crate::error::SymphonyError;
use crate::tracker::model::Issue;
use crate::tracker::IssueTracker;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use serde_json::json;

pub struct GithubTracker {
    client: reqwest::Client,
    owner: String,
    repo: String,
    project_number: i64,
    endpoint: String,
}

impl GithubTracker {
    pub fn new(config: &ServiceConfig) -> Result<Self, SymphonyError> {
        let api_key = config
            .tracker_api_key()
            .ok_or(SymphonyError::MissingTrackerApiKey)?;
        let project = config
            .tracker_project_slug()
            .ok_or(SymphonyError::MissingTrackerProjectSlug)?;
        let project_number = config
            .tracker_project_number()
            .ok_or_else(|| SymphonyError::InvalidConfiguration(
                "tracker.project_number is required for github projects".into(),
            ))?;
        let parts: Vec<&str> = project.split('/').collect();
        if parts.len() != 2 {
            return Err(SymphonyError::InvalidConfiguration(
                "tracker.project_slug must be owner/repo".into(),
            ));
        }
        let mut headers = HeaderMap::new();
        let auth = HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|e| SymphonyError::InvalidConfiguration(e.to_string()))?;
        headers.insert(AUTHORIZATION, auth);
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("symphonie/0.1.0"),
        );
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| SymphonyError::TrackerApiRequest(e.to_string()))?;
        Ok(Self {
            client,
            owner: parts[0].to_string(),
            repo: parts[1].to_string(),
            project_number,
            endpoint: config.tracker_endpoint(),
        })
    }

    async fn graphql_query(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<serde_json::Value, SymphonyError> {
        let body = json!({ "query": query, "variables": variables });
        let resp = self
            .client
            .post(format!("{}/graphql", self.endpoint))
            .json(&body)
            .send()
            .await
            .map_err(|e| SymphonyError::TrackerApiRequest(e.to_string()))?;
        let status = resp.status();
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SymphonyError::TrackerMalformedPayload(e.to_string()))?;
        if let Some(errors) = json.get("errors") {
            return Err(SymphonyError::TrackerApiStatus(format!(
                "GraphQL errors: {}",
                errors
            )));
        }
        if !status.is_success() {
            return Err(SymphonyError::TrackerApiStatus(format!(
                "HTTP {}: {}",
                status, json
            )));
        }
        Ok(json["data"].clone())
    }

    async fn fetch_project_items(&self) -> Result<Vec<serde_json::Value>, SymphonyError> {
        // Determine if owner is an organization or a user
        let org_query = r#"
            query($owner: String!, $projectNumber: Int!) {
              organization(login: $owner) {
                projectV2(number: $projectNumber) { id }
              }
            }
        "#;
        let org_data = self
            .graphql_query(org_query, json!({"owner": &self.owner, "projectNumber": self.project_number}))
            .await?;
        let is_org = org_data
            .get("organization")
            .and_then(|o| o.get("projectV2"))
            .is_some();

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
        if content.get("number").is_none() {
            return None;
        }

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
            blocked_by: vec![],
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
    async fn fetch_candidate_issues(&self) -> Result<Vec<Issue>, SymphonyError> {
        let items = self.fetch_project_items().await?;
        let issues: Vec<Issue> = items.iter().filter_map(|i| self.normalize_item(i)).collect();
        Ok(issues)
    }

    async fn fetch_issues_by_states(&self, states: &[String]) -> Result<Vec<Issue>, SymphonyError> {
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

    async fn fetch_issue_states_by_ids(&self, ids: &[String]) -> Result<Vec<Issue>, SymphonyError> {
        let items = self.fetch_project_items().await?;
        let issues: Vec<Issue> = items
            .iter()
            .filter_map(|i| self.normalize_item(i))
            .filter(|issue| ids.contains(&issue.id))
            .collect();
        Ok(issues)
    }
}
