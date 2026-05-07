use crate::config::resolver;
use crate::error::SymphonyError;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ServiceConfig {
    raw: serde_yaml::Mapping,
    workflow_dir: PathBuf,
    pub prompt_template: String,
}

impl ServiceConfig {
    pub fn new(raw: serde_yaml::Mapping, workflow_dir: PathBuf, prompt_template: String) -> Self {
        Self { raw, workflow_dir, prompt_template }
    }

    pub fn raw(&self) -> &serde_yaml::Mapping {
        &self.raw
    }

    fn tracker(&self) -> Option<&serde_yaml::Mapping> {
        self.raw.get("tracker").and_then(|v| v.as_mapping())
    }

    fn polling(&self) -> Option<&serde_yaml::Mapping> {
        self.raw.get("polling").and_then(|v| v.as_mapping())
    }

    fn workspace(&self) -> Option<&serde_yaml::Mapping> {
        self.raw.get("workspace").and_then(|v| v.as_mapping())
    }

    fn hooks(&self) -> Option<&serde_yaml::Mapping> {
        self.raw.get("hooks").and_then(|v| v.as_mapping())
    }

    fn agent(&self) -> Option<&serde_yaml::Mapping> {
        self.raw.get("agent").and_then(|v| v.as_mapping())
    }

    fn codex(&self) -> Option<&serde_yaml::Mapping> {
        self.raw.get("codex").and_then(|v| v.as_mapping())
    }

    pub fn tracker_kind(&self) -> Option<String> {
        self.tracker()
            .and_then(|m| resolver::get_string(m, "kind"))
    }

    pub fn tracker_endpoint(&self) -> String {
        self.tracker()
            .and_then(|m| resolver::get_string(m, "endpoint"))
            .unwrap_or_else(|| "https://api.github.com".to_string())
    }

    pub fn tracker_api_key(&self) -> Option<String> {
        self.tracker()
            .and_then(|m| resolver::get_string(m, "api_key"))
            .map(|s| resolver::resolve_value(&s))
            .filter(|s| !s.is_empty())
    }

    pub fn tracker_project_slug(&self) -> Option<String> {
        self.tracker()
            .and_then(|m| resolver::get_string(m, "project_slug"))
    }

    pub fn tracker_project_number(&self) -> Option<i64> {
        self.tracker()
            .and_then(|m| resolver::get_i64(m, "project_number"))
    }

    pub fn active_states(&self) -> Vec<String> {
        self.tracker()
            .and_then(|m| resolver::get_str_list(m, "active_states"))
            .unwrap_or_else(|| vec!["Todo".into(), "In Progress".into()])
            .into_iter()
            .map(|s| s.to_lowercase())
            .collect()
    }

    pub fn terminal_states(&self) -> Vec<String> {
        self.tracker()
            .and_then(|m| resolver::get_str_list(m, "terminal_states"))
            .unwrap_or_else(|| {
                vec![
                    "Closed".into(),
                    "Cancelled".into(),
                    "Canceled".into(),
                    "Duplicate".into(),
                    "Done".into(),
                ]
            })
            .into_iter()
            .map(|s| s.to_lowercase())
            .collect()
    }

    pub fn poll_interval_ms(&self) -> u64 {
        self.polling()
            .and_then(|m| resolver::get_i64(m, "interval_ms"))
            .unwrap_or(30000)
            .max(1000) as u64
    }

    pub fn workspace_root(&self) -> Result<PathBuf, SymphonyError> {
        let raw = self
            .workspace()
            .and_then(|m| resolver::get_string(m, "root"))
            .unwrap_or_else(|| {
                std::env::temp_dir()
                    .join("symphony_workspaces")
                    .to_string_lossy()
                    .to_string()
            });
        let resolved = resolver::resolve_path(&raw, &self.workflow_dir)?;
        Ok(resolved)
    }

    pub fn hook_script(&self, name: &str) -> Option<String> {
        self.hooks().and_then(|m| resolver::get_string(m, name))
    }

    pub fn hook_timeout_ms(&self) -> u64 {
        self.hooks()
            .and_then(|m| resolver::get_i64(m, "timeout_ms"))
            .unwrap_or(60000)
            .max(0) as u64
    }

    pub fn max_concurrent_agents(&self) -> usize {
        self.agent()
            .and_then(|m| resolver::get_i64(m, "max_concurrent_agents"))
            .unwrap_or(10)
            .max(1) as usize
    }

    pub fn max_turns(&self) -> u32 {
        self.agent()
            .and_then(|m| resolver::get_i64(m, "max_turns"))
            .unwrap_or(20)
            .max(1) as u32
    }

    pub fn max_retry_backoff_ms(&self) -> u64 {
        self.agent()
            .and_then(|m| resolver::get_i64(m, "max_retry_backoff_ms"))
            .unwrap_or(300000)
            .max(1000) as u64
    }

    pub fn max_concurrent_agents_by_state(&self) -> HashMap<String, usize> {
        let mut map = HashMap::new();
        if let Some(agent_map) = self.agent() {
            if let Some(state_map) = agent_map
                .get("max_concurrent_agents_by_state")
                .and_then(|v| v.as_mapping())
            {
                for (k, v) in state_map {
                    if let (Some(key), Some(val)) = (k.as_str(), v.as_i64()) {
                        if val > 0 {
                            map.insert(key.to_lowercase(), val as usize);
                        }
                    }
                }
            }
        }
        map
    }

    pub fn codex_command(&self) -> String {
        self.codex()
            .and_then(|m| resolver::get_string(m, "command"))
            .unwrap_or_else(|| "opencode run".to_string())
    }

    pub fn codex_turn_timeout_ms(&self) -> u64 {
        self.codex()
            .and_then(|m| resolver::get_i64(m, "turn_timeout_ms"))
            .unwrap_or(3600000)
            .max(0) as u64
    }

    pub fn codex_read_timeout_ms(&self) -> u64 {
        self.codex()
            .and_then(|m| resolver::get_i64(m, "read_timeout_ms"))
            .unwrap_or(5000)
            .max(0) as u64
    }

    pub fn codex_stall_timeout_ms(&self) -> i64 {
        self.codex()
            .and_then(|m| resolver::get_i64(m, "stall_timeout_ms"))
            .unwrap_or(300000)
    }

    pub fn validate_for_dispatch(&self) -> Result<(), SymphonyError> {
        let kind = self.tracker_kind().ok_or(SymphonyError::InvalidConfiguration(
            "tracker.kind is required".into(),
        ))?;
        if kind != "github" {
            return Err(SymphonyError::UnsupportedTrackerKind(kind));
        }
        if self.tracker_api_key().is_none() {
            return Err(SymphonyError::MissingTrackerApiKey);
        }
        if self.tracker_project_slug().is_none() {
            return Err(SymphonyError::MissingTrackerProjectSlug);
        }
        if self.tracker_project_number().is_none() {
            return Err(SymphonyError::InvalidConfiguration(
                "tracker.project_number is required for github projects".into(),
            ));
        }
        let cmd = self.codex_command();
        if cmd.trim().is_empty() {
            return Err(SymphonyError::InvalidConfiguration(
                "codex.command is empty".into(),
            ));
        }
        Ok(())
    }
}
