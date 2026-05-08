use std::path::PathBuf;
use sympheo::config::typed::ServiceConfig;
use sympheo::tracker::github::GithubTracker;
use sympheo::tracker::IssueTracker;
use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

fn make_config(endpoint: &str) -> ServiceConfig {
    let mut raw = serde_json::Map::<String, serde_json::Value>::new();
    let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
    tracker.insert(
        "kind".into(),
        serde_json::Value::String("github".into()),
    );
    tracker.insert(
        "api_key".into(),
        serde_json::Value::String("test-key".into()),
    );
    tracker.insert(
        "project_slug".into(),
        serde_json::Value::String("owner/repo".into()),
    );
    tracker.insert(
        "project_number".into(),
        serde_json::Value::Number(1.into()),
    );
    tracker.insert(
        "endpoint".into(),
        serde_json::Value::String(endpoint.into()),
    );
    raw.insert(
        "tracker".into(),
        serde_json::Value::Object(tracker),
    );
    ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into())
}

fn org_project_items_response() -> serde_json::Value {
    serde_json::json!({
        "data": {
            "organization": {
                "projectV2": {
                    "items": {
                        "nodes": [
                            {
                                "id": "item-1",
                                "content": {
                                    "id": "issue-1",
                                    "number": 1,
                                    "title": "Issue One",
                                    "body": "Description one",
                                    "state": "OPEN",
                                    "labels": { "nodes": [{ "name": "bug" }] },
                                    "createdAt": "2024-01-15T10:00:00Z",
                                    "updatedAt": "2024-01-16T12:00:00Z",
                                    "url": "https://github.com/owner/repo/issues/1",
                                    "repository": { "name": "repo", "owner": { "login": "owner" } }
                                },
                                "fieldValues": {
                                    "nodes": [
                                        { "field": { "name": "Status" }, "name": "In Progress" }
                                    ]
                                }
                            },
                            {
                                "id": "item-2",
                                "content": {
                                    "id": "issue-2",
                                    "number": 2,
                                    "title": "Issue Two",
                                    "body": null,
                                    "state": "CLOSED",
                                    "labels": { "nodes": [] },
                                    "createdAt": "2024-01-10T08:00:00Z",
                                    "updatedAt": "2024-01-11T09:00:00Z",
                                    "url": "https://github.com/owner/repo/issues/2",
                                    "repository": { "name": "repo", "owner": { "login": "owner" } }
                                },
                                "fieldValues": {
                                    "nodes": []
                                }
                            }
                        ],
                        "pageInfo": { "hasNextPage": false, "endCursor": null }
                    }
                }
            }
        }
    })
}

fn org_exists_response() -> serde_json::Value {
    serde_json::json!({
        "data": {
            "organization": {
                "projectV2": { "id": "proj-1" }
            }
        }
    })
}

#[tokio::test]
async fn test_fetch_candidate_issues() {
    let mock_server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(org_exists_response()))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(org_project_items_response()))
        .mount(&mock_server)
        .await;

    let config = make_config(&mock_server.uri());
    let tracker = GithubTracker::new(&config).unwrap();
    let issues = tracker.fetch_candidate_issues().await.unwrap();

    assert_eq!(issues.len(), 2);
    assert_eq!(issues[0].id, "1");
    assert_eq!(issues[0].identifier, "REPO-1");
    assert_eq!(issues[0].title, "Issue One");
    assert_eq!(issues[0].state, "in progress");
    assert_eq!(issues[0].labels, vec!["bug"]);
    assert_eq!(issues[1].id, "2");
    assert_eq!(issues[1].state, "closed");
}

#[tokio::test]
async fn test_fetch_issues_by_states() {
    let mock_server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(org_exists_response()))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(org_project_items_response()))
        .mount(&mock_server)
        .await;

    let config = make_config(&mock_server.uri());
    let tracker = GithubTracker::new(&config).unwrap();
    let issues = tracker.fetch_issues_by_states(&["closed".into()]).await.unwrap();

    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].id, "2");
    assert_eq!(issues[0].state, "closed");
}

#[tokio::test]
async fn test_fetch_issues_by_states_empty() {
    let mock_server = MockServer::start().await;

    let config = make_config(&mock_server.uri());
    let tracker = GithubTracker::new(&config).unwrap();
    let issues = tracker.fetch_issues_by_states(&[]).await.unwrap();

    assert!(issues.is_empty());
}

#[tokio::test]
async fn test_fetch_issue_states_by_ids() {
    let mock_server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(org_exists_response()))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(org_project_items_response()))
        .mount(&mock_server)
        .await;

    let config = make_config(&mock_server.uri());
    let tracker = GithubTracker::new(&config).unwrap();
    let issues = tracker.fetch_issue_states_by_ids(&["1".into()]).await.unwrap();

    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].id, "1");
}

#[tokio::test]
async fn test_graphql_error_response() {
    let mock_server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "errors": [{ "message": "Bad credentials" }]
        })))
        .mount(&mock_server)
        .await;

    let config = make_config(&mock_server.uri());
    let tracker = GithubTracker::new(&config).unwrap();
    let result = tracker.fetch_candidate_issues().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_move_issue_state_e2e() {
    let mock_server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {
                "organization": { "projectV2": { "id": "proj-123" } }
            }
        })))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {
                "node": {
                    "fields": {
                        "nodes": [
                            {
                                "id": "field-1",
                                "name": "Status",
                                "options": [
                                    { "id": "opt-1", "name": "Todo" },
                                    { "id": "opt-2", "name": "Done" }
                                ]
                            }
                        ]
                    }
                }
            }
        })))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {
                "updateProjectV2ItemFieldValue": {
                    "projectV2Item": { "id": "item-123" }
                }
            }
        })))
        .mount(&mock_server)
        .await;

    let config = make_config(&mock_server.uri());
    let tracker = GithubTracker::new(&config).unwrap();
    let issue = sympheo::tracker::model::Issue {
        id: "issue-1".into(),
        project_item_id: Some("item-123".into()),
        ..Default::default()
    };
    assert!(tracker.move_issue_state(&issue, "Done").await.is_ok());
}

#[tokio::test]
async fn test_add_comment_e2e() {
    let mock_server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {
                "addComment": {
                    "commentEdge": { "node": { "id": "comment-123" } }
                }
            }
        })))
        .mount(&mock_server)
        .await;

    let config = make_config(&mock_server.uri());
    let tracker = GithubTracker::new(&config).unwrap();
    let issue = sympheo::tracker::model::Issue {
        id: "issue-1".into(),
        node_id: Some("node-123".into()),
        ..Default::default()
    };
    assert!(tracker.add_comment(&issue, "Great work!").await.is_ok());
}

#[tokio::test]
async fn test_update_issue_body_e2e() {
    let mock_server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {
                "updateIssue": {
                    "issue": { "id": "issue-123" }
                }
            }
        })))
        .mount(&mock_server)
        .await;

    let config = make_config(&mock_server.uri());
    let tracker = GithubTracker::new(&config).unwrap();
    let issue = sympheo::tracker::model::Issue {
        id: "issue-1".into(),
        node_id: Some("node-123".into()),
        ..Default::default()
    };
    assert!(tracker.update_issue_body(&issue, "Updated body text").await.is_ok());
}

#[tokio::test]
async fn test_fetch_candidate_issues_user_path() {
    let mock_server = MockServer::start().await;

    // First query: org check returns no organization at all
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .and(matchers::body_string_contains("projectV2(number: $projectNumber) { id }"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "organization": null }
        })))
        .mount(&mock_server)
        .await;

    // Second query: user project items
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {
                "user": {
                    "projectV2": {
                        "items": {
                            "nodes": [
                                {
                                    "id": "item-1",
                                    "content": {
                                        "number": 1,
                                        "title": "User Issue",
                                        "body": null,
                                        "state": "OPEN",
                                        "labels": { "nodes": [] },
                                        "createdAt": "2024-01-15T10:00:00Z",
                                        "updatedAt": "2024-01-16T12:00:00Z",
                                        "url": "https://github.com/owner/repo/issues/1",
                                        "repository": { "name": "repo", "owner": { "login": "owner" } }
                                    },
                                    "fieldValues": { "nodes": [] }
                                }
                            ],
                            "pageInfo": { "hasNextPage": false, "endCursor": null }
                        }
                    }
                }
            }
        })))
        .mount(&mock_server)
        .await;

    let config = make_config(&mock_server.uri());
    let tracker = GithubTracker::new(&config).unwrap();
    let issues = tracker.fetch_candidate_issues().await.unwrap();
    eprintln!("Issues: {:?}", issues);
    for req in mock_server.received_requests().await.unwrap() {
        eprintln!("Request: {:?}", req);
    }
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].title, "User Issue");
}

#[tokio::test]
async fn test_fetch_candidate_issues_pagination() {
    let mock_server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "organization": { "projectV2": { "id": "proj-1" } } }
        })))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .and(matchers::body_string_contains("projectV2(number: $projectNumber) { id }"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "organization": { "projectV2": { "id": "proj-1" } } }
        })))
        .mount(&mock_server)
        .await;

    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .and(matchers::body_string_contains("\"after\":null"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {
                "organization": {
                    "projectV2": {
                        "items": {
                            "nodes": [
                                {
                                    "id": "item-1",
                                    "content": {
                                        "number": 1,
                                        "title": "Page 1",
                                        "body": null,
                                        "state": "OPEN",
                                        "labels": { "nodes": [] },
                                        "createdAt": "2024-01-15T10:00:00Z",
                                        "updatedAt": "2024-01-16T12:00:00Z",
                                        "url": "https://github.com/owner/repo/issues/1",
                                        "repository": { "name": "repo", "owner": { "login": "owner" } }
                                    },
                                    "fieldValues": { "nodes": [] }
                                }
                            ],
                            "pageInfo": { "hasNextPage": true, "endCursor": "cursor1" }
                        }
                    }
                }
            }
        })))
        .mount(&mock_server)
        .await;

    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .and(matchers::body_string_contains("\"after\":\"cursor1\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {
                "organization": {
                    "projectV2": {
                        "items": {
                            "nodes": [
                                {
                                    "id": "item-2",
                                    "content": {
                                        "number": 2,
                                        "title": "Page 2",
                                        "body": null,
                                        "state": "OPEN",
                                        "labels": { "nodes": [] },
                                        "createdAt": "2024-01-15T10:00:00Z",
                                        "updatedAt": "2024-01-16T12:00:00Z",
                                        "url": "https://github.com/owner/repo/issues/2",
                                        "repository": { "name": "repo", "owner": { "login": "owner" } }
                                    },
                                    "fieldValues": { "nodes": [] }
                                }
                            ],
                            "pageInfo": { "hasNextPage": false, "endCursor": null }
                        }
                    }
                }
            }
        })))
        .mount(&mock_server)
        .await;

    let config = make_config(&mock_server.uri());
    let tracker = GithubTracker::new(&config).unwrap();
    let issues = tracker.fetch_candidate_issues().await.unwrap();
    eprintln!("Pagination issues: {:?}", issues);
    for req in mock_server.received_requests().await.unwrap() {
        eprintln!("Pagination request: {:?} {:?}", req.method, req.url);
    }
    assert_eq!(issues.len(), 2);
    assert_eq!(issues[0].title, "Page 1");
    assert_eq!(issues[1].title, "Page 2");
}

#[tokio::test]
async fn test_http_error_response() {
    let mock_server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .and(matchers::path("/graphql"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "message": "Unauthorized"
        })))
        .mount(&mock_server)
        .await;

    let config = make_config(&mock_server.uri());
    let tracker = GithubTracker::new(&config).unwrap();
    let result = tracker.fetch_candidate_issues().await;
    assert!(result.is_err());
}
