//! Mock execution backend for tests and dry-runs.
//!
//! Triggered by `cli.command = "mock-cli"` in WORKFLOW.md. Reads a YAML / JSON
//! script of `AgentEvent`s from `cli.options.script` (a path relative to the
//! workspace) and emits them in order with the `delay_ms` between each. No
//! subprocess is spawned; no tokens are consumed.
//!
//! Intended for two use cases:
//!
//! 1. **E2E tests** — drive the orchestrator through a complete worker
//!    lifecycle (turn loop, retries, stalls, reconciliation) without a real
//!    opencode invocation. Fixtures live in `tests/fixtures/mock-runs/`.
//! 2. **Dry-runs** — `WORKFLOW.md` operator can swap `opencode run` for
//!    `mock-cli` to validate that the orchestrator + skills + workflow
//!    plumbing works end-to-end before spending tokens on a real run.
//!
//! Script format (one document per turn):
//!
//! ```yaml
//! events:
//!   - type: step_start
//!     session_id: sess-1
//!     message_id: msg-1
//!   - type: text
//!     message_id: msg-1
//!     text: "Hello"
//!     delay_ms: 50
//!   - type: step_finish
//!     message_id: msg-2
//!     reason: stop
//! ```
//!
//! Spec relation: §10.6 "OpenCode adapter" is the reference adapter; this is
//! a parallel adapter that satisfies the same contract surface (start_session
//! semantics, run_turn returns TurnResult, no stop_session needed). It is
//! documented as an extension in docs/extensions.md.

use crate::agent::backend::AgentBackend;
use crate::agent::parser::{
    AgentEvent, StepFinishPart, StepStartPart, TextPart, TokenInfo, TurnResult,
};
use crate::config::typed::ServiceConfig;
use crate::error::SympheoError;
use crate::tracker::model::Issue;
use async_trait::async_trait;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc::Sender;
use tokio::time::Duration;

#[derive(Debug, Clone, Deserialize)]
pub struct MockScript {
    pub events: Vec<MockScriptEvent>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MockScriptEvent {
    StepStart {
        #[serde(default)]
        session_id: String,
        #[serde(default)]
        message_id: String,
        #[serde(default)]
        delay_ms: u64,
    },
    Text {
        #[serde(default)]
        message_id: String,
        text: String,
        #[serde(default)]
        delay_ms: u64,
    },
    StepFinish {
        #[serde(default)]
        message_id: String,
        #[serde(default = "default_finish_reason")]
        reason: String,
        #[serde(default)]
        input_tokens: u64,
        #[serde(default)]
        output_tokens: u64,
        #[serde(default)]
        delay_ms: u64,
    },
    /// Inject a delay without emitting an event — useful for stall tests.
    Sleep { delay_ms: u64 },
}

fn default_finish_reason() -> String {
    "stop".to_string()
}

pub struct MockBackend {
    script_path: PathBuf,
}

impl MockBackend {
    pub fn new(config: &ServiceConfig) -> Result<Self, SympheoError> {
        let opts = config.cli_options();
        let script_path = opts
            .get("script")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                SympheoError::InvalidConfiguration(
                    "mock-cli requires cli.options.script (path to YAML/JSON script)".into(),
                )
            })?
            .to_string();
        Ok(Self {
            script_path: PathBuf::from(script_path),
        })
    }

    fn resolve_script_path(&self, workspace_path: &Path) -> PathBuf {
        if self.script_path.is_absolute() {
            self.script_path.clone()
        } else {
            workspace_path.join(&self.script_path)
        }
    }
}

#[async_trait]
impl AgentBackend for MockBackend {
    async fn run_turn(
        &self,
        issue: &Issue,
        _prompt: &str,
        _session_id: Option<&str>,
        workspace_path: &Path,
        cancelled: Arc<AtomicBool>,
        event_tx: Sender<AgentEvent>,
    ) -> Result<TurnResult, SympheoError> {
        let path = self.resolve_script_path(workspace_path);
        let raw = tokio::fs::read_to_string(&path).await.map_err(|e| {
            SympheoError::AgentRunnerError(format!(
                "mock script read failed at {}: {e}",
                path.display()
            ))
        })?;
        let script: MockScript = if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("json"))
            .unwrap_or(false)
        {
            serde_json::from_str(&raw).map_err(|e| {
                SympheoError::OutputParseError(format!("mock script JSON parse: {e}"))
            })?
        } else {
            // YAML default
            serde_saphyr::from_str(&raw).map_err(|e| {
                SympheoError::OutputParseError(format!("mock script YAML parse: {e}"))
            })?
        };

        let mut current_session = issue.id.clone();
        let mut current_turn = "mock-turn-1".to_string();
        let mut accumulated_text = String::new();
        let mut tokens: Option<TokenInfo> = None;
        let mut success = false;

        for ev in script.events {
            if cancelled.load(Ordering::Relaxed) {
                return Err(SympheoError::TurnCancelled);
            }
            // Apply delay BEFORE emitting (so events space out as scripted).
            let delay = match &ev {
                MockScriptEvent::StepStart { delay_ms, .. }
                | MockScriptEvent::Text { delay_ms, .. }
                | MockScriptEvent::StepFinish { delay_ms, .. }
                | MockScriptEvent::Sleep { delay_ms } => *delay_ms,
            };
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }

            let agent_event = match ev {
                MockScriptEvent::StepStart {
                    session_id,
                    message_id,
                    ..
                } => {
                    let sess = if session_id.is_empty() {
                        current_session.clone()
                    } else {
                        current_session = session_id.clone();
                        session_id
                    };
                    let mid = if message_id.is_empty() {
                        current_turn.clone()
                    } else {
                        current_turn = message_id.clone();
                        message_id
                    };
                    Some(AgentEvent::StepStart {
                        session_id: sess.clone(),
                        timestamp: 0,
                        part: StepStartPart {
                            id: format!("mock-step-{mid}"),
                            message_id: mid,
                            session_id: sess,
                            part_type: "step".into(),
                        },
                    })
                }
                MockScriptEvent::Text {
                    message_id, text, ..
                } => {
                    let mid = if message_id.is_empty() {
                        current_turn.clone()
                    } else {
                        current_turn = message_id.clone();
                        message_id
                    };
                    accumulated_text.push_str(&text);
                    Some(AgentEvent::Text {
                        session_id: current_session.clone(),
                        timestamp: 0,
                        part: TextPart {
                            id: format!("mock-text-{mid}"),
                            message_id: mid,
                            session_id: current_session.clone(),
                            part_type: "text".into(),
                            text,
                            time: None,
                        },
                    })
                }
                MockScriptEvent::StepFinish {
                    message_id,
                    reason,
                    input_tokens,
                    output_tokens,
                    ..
                } => {
                    let mid = if message_id.is_empty() {
                        current_turn.clone()
                    } else {
                        message_id
                    };
                    let total = input_tokens + output_tokens;
                    if input_tokens > 0 || output_tokens > 0 {
                        tokens = Some(TokenInfo {
                            total,
                            input: input_tokens,
                            output: output_tokens,
                            reasoning: 0,
                            cache: None,
                        });
                    }
                    success = reason == "stop" || reason == "tool-calls";
                    Some(AgentEvent::StepFinish {
                        session_id: current_session.clone(),
                        timestamp: 0,
                        part: StepFinishPart {
                            id: format!("mock-finish-{mid}"),
                            message_id: mid,
                            session_id: current_session.clone(),
                            part_type: "finish".into(),
                            reason,
                            tokens: tokens.clone(),
                            cost: None,
                        },
                    })
                }
                MockScriptEvent::Sleep { .. } => None,
            };

            if let Some(event) = agent_event {
                let _ = event_tx.send(event).await;
            }
        }

        Ok(TurnResult {
            session_id: current_session,
            turn_id: current_turn,
            success,
            text: accumulated_text,
            tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn config_with_script(workspace_root: &Path, script: &str) -> ServiceConfig {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut workspace = serde_json::Map::<String, serde_json::Value>::new();
        workspace.insert(
            "root".into(),
            serde_json::Value::String(workspace_root.to_string_lossy().to_string()),
        );
        raw.insert("workspace".into(), serde_json::Value::Object(workspace));

        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        cli.insert(
            "command".into(),
            serde_json::Value::String("mock-cli".into()),
        );
        let mut opts = serde_json::Map::<String, serde_json::Value>::new();
        opts.insert("script".into(), serde_json::Value::String(script.into()));
        cli.insert("options".into(), serde_json::Value::Object(opts));
        raw.insert("cli".into(), serde_json::Value::Object(cli));

        ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into())
    }

    fn unique_tmp(suffix: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("sympheo_mock_{}_{}", suffix, ts))
    }

    fn dummy_issue() -> Issue {
        Issue {
            id: "I_kwDO123".into(),
            identifier: "repo#1".into(),
            title: "test".into(),
            description: None,
            priority: None,
            state: "todo".into(),
            branch_name: None,
            url: None,
            labels: vec![],
            blocked_by: vec![],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_mock_backend_missing_script_config() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        cli.insert(
            "command".into(),
            serde_json::Value::String("mock-cli".into()),
        );
        raw.insert("cli".into(), serde_json::Value::Object(cli));
        let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
        let result = MockBackend::new(&config);
        assert!(matches!(result, Err(SympheoError::InvalidConfiguration(_))));
    }

    #[tokio::test]
    async fn test_mock_backend_success_yaml() {
        let tmp = unique_tmp("ok");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let script = r#"
events:
  - type: step_start
    session_id: sess-1
    message_id: msg-1
  - type: text
    message_id: msg-1
    text: "Hello, world"
  - type: step_finish
    message_id: msg-1
    reason: stop
    input_tokens: 10
    output_tokens: 20
"#;
        std::fs::write(tmp.join("script.yaml"), script).unwrap();
        let config = config_with_script(&tmp, "script.yaml");
        let backend = MockBackend::new(&config).unwrap();
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(64);
        let result = backend
            .run_turn(
                &dummy_issue(),
                "prompt",
                None,
                &tmp,
                Arc::new(AtomicBool::new(false)),
                event_tx,
            )
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.text, "Hello, world");
        assert_eq!(result.session_id, "sess-1");
        assert!(result.tokens.is_some());
        let toks = result.tokens.unwrap();
        assert_eq!(toks.input, 10);
        assert_eq!(toks.output, 20);
        assert_eq!(toks.total, 30);
        // 3 events emitted
        let mut count = 0;
        while event_rx.try_recv().is_ok() {
            count += 1;
        }
        assert_eq!(count, 3);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_mock_backend_empty_run_no_artifact() {
        // Reproduce the 2026-05-09 incident shape: agent claims reason=stop
        // without doing anything. The mock backend reports success=true (it's
        // not the orchestrator's job to validate per §11.5/§15.1) — the
        // dashboard / hooks / skills are responsible for catching this.
        let tmp = unique_tmp("empty");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let script = r#"
events:
  - type: step_start
    session_id: sess-1
    message_id: msg-1
  - type: step_finish
    message_id: msg-1
    reason: stop
"#;
        std::fs::write(tmp.join("script.yaml"), script).unwrap();
        let config = config_with_script(&tmp, "script.yaml");
        let backend = MockBackend::new(&config).unwrap();
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
        let result = backend
            .run_turn(
                &dummy_issue(),
                "prompt",
                None,
                &tmp,
                Arc::new(AtomicBool::new(false)),
                event_tx,
            )
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.text, "");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_mock_backend_cancellation() {
        let tmp = unique_tmp("cancel");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let script = r#"
events:
  - type: step_start
    session_id: sess-1
    message_id: msg-1
    delay_ms: 50
  - type: sleep
    delay_ms: 5000
  - type: step_finish
    reason: stop
"#;
        std::fs::write(tmp.join("script.yaml"), script).unwrap();
        let config = config_with_script(&tmp, "script.yaml");
        let backend = MockBackend::new(&config).unwrap();
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_setter = cancelled.clone();
        // Fire a cancel after 100ms — the mock should observe it before the
        // 5-second sleep completes and return TurnCancelled.
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancelled_setter.store(true, Ordering::Relaxed);
        });
        let result = backend
            .run_turn(&dummy_issue(), "prompt", None, &tmp, cancelled, event_tx)
            .await;
        assert!(matches!(result, Err(SympheoError::TurnCancelled)));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_mock_backend_json_format() {
        let tmp = unique_tmp("json");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let script = serde_json::json!({
            "events": [
                {"type": "step_start", "session_id": "j-1", "message_id": "m-1"},
                {"type": "step_finish", "message_id": "m-1", "reason": "stop"}
            ]
        });
        std::fs::write(
            tmp.join("script.json"),
            serde_json::to_string(&script).unwrap(),
        )
        .unwrap();
        let config = config_with_script(&tmp, "script.json");
        let backend = MockBackend::new(&config).unwrap();
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
        let result = backend
            .run_turn(
                &dummy_issue(),
                "prompt",
                None,
                &tmp,
                Arc::new(AtomicBool::new(false)),
                event_tx,
            )
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.session_id, "j-1");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_mock_backend_missing_script_file() {
        let tmp = unique_tmp("missing");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let config = config_with_script(&tmp, "does-not-exist.yaml");
        let backend = MockBackend::new(&config).unwrap();
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
        let result = backend
            .run_turn(
                &dummy_issue(),
                "prompt",
                None,
                &tmp,
                Arc::new(AtomicBool::new(false)),
                event_tx,
            )
            .await;
        assert!(matches!(result, Err(SympheoError::AgentRunnerError(_))));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
