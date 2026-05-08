use crate::config::resolver;
use crate::error::SympheoError;
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

    pub fn daytona(&self) -> Option<&serde_yaml::Mapping> {
        self.raw.get("daytona").and_then(|v| v.as_mapping())
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

    pub fn workspace_root(&self) -> Result<PathBuf, SympheoError> {
        let raw = self
            .workspace()
            .and_then(|m| resolver::get_string(m, "root"))
            .unwrap_or_else(|| {
                std::env::temp_dir()
                    .join("sympheo_workspaces")
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

    // Daytona helpers
    pub fn daytona_enabled(&self) -> bool {
        self.daytona()
            .and_then(|m| m.get("enabled").and_then(|v| v.as_bool()))
            .unwrap_or(false)
    }

    pub fn daytona_api_key(&self) -> Option<String> {
        self.daytona()
            .and_then(|m| resolver::get_string(m, "api_key"))
            .map(|s| resolver::resolve_value(&s))
            .filter(|s| !s.is_empty())
    }

    pub fn daytona_api_url(&self) -> String {
        self.daytona()
            .and_then(|m| resolver::get_string(m, "api_url"))
            .unwrap_or_else(|| "https://api.daytona.io".to_string())
    }

    pub fn daytona_target(&self) -> String {
        self.daytona()
            .and_then(|m| resolver::get_string(m, "target"))
            .unwrap_or_else(|| "us".to_string())
    }

    pub fn daytona_image(&self) -> Option<String> {
        self.daytona()
            .and_then(|m| resolver::get_string(m, "image"))
            .filter(|s| !s.is_empty())
    }

    pub fn daytona_timeout_sec(&self) -> u64 {
        self.daytona()
            .and_then(|m| resolver::get_i64(m, "timeout_sec"))
            .unwrap_or(3600)
            .max(30) as u64
    }

    pub fn daytona_env(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        if let Some(daytona_map) = self.daytona() {
            if let Some(env_map) = daytona_map.get("env").and_then(|v| v.as_mapping()) {
                for (k, v) in env_map {
                    if let (Some(key), Some(val)) = (k.as_str(), v.as_str()) {
                        map.insert(key.to_string(), resolver::resolve_value(val));
                    }
                }
            }
        }
        map
    }

    pub fn validate_for_dispatch(&self) -> Result<(), SympheoError> {
        let kind = self.tracker_kind().ok_or(SympheoError::InvalidConfiguration(
            "tracker.kind is required".into(),
        ))?;
        if kind != "github" {
            return Err(SympheoError::UnsupportedTrackerKind(kind));
        }
        if self.tracker_api_key().is_none() {
            return Err(SympheoError::MissingTrackerApiKey);
        }
        if self.tracker_project_slug().is_none() {
            return Err(SympheoError::MissingTrackerProjectSlug);
        }
        if self.tracker_project_number().is_none() {
            return Err(SympheoError::InvalidConfiguration(
                "tracker.project_number is required for github projects".into(),
            ));
        }
        let cmd = self.codex_command();
        if cmd.trim().is_empty() {
            return Err(SympheoError::InvalidConfiguration(
                "codex.command is empty".into(),
            ));
        }
        if self.daytona_enabled()
            && self.daytona_api_key().is_none() {
                return Err(SympheoError::InvalidConfiguration(
                    "daytona.api_key is required when backend is enabled".into(),
                ));
            }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn empty_config() -> ServiceConfig {
        ServiceConfig::new(serde_yaml::Mapping::new(), PathBuf::from("/tmp"), "".into())
    }

    fn config_with(raw: serde_yaml::Mapping) -> ServiceConfig {
        ServiceConfig::new(raw, PathBuf::from("/tmp"), "prompt".into())
    }

    #[test]
    fn test_raw_accessor() {
        let config = empty_config();
        assert!(config.raw().is_empty());
    }

    #[test]
    fn test_tracker_kind_missing() {
        assert_eq!(empty_config().tracker_kind(), None);
    }

    #[test]
    fn test_tracker_kind_present() {
        let mut raw = serde_yaml::Mapping::new();
        let mut tracker = serde_yaml::Mapping::new();
        tracker.insert(
            serde_yaml::Value::String("kind".into()),
            serde_yaml::Value::String("github".into()),
        );
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(tracker),
        );
        assert_eq!(config_with(raw).tracker_kind(), Some("github".to_string()));
    }

    #[test]
    fn test_tracker_endpoint_default() {
        assert_eq!(empty_config().tracker_endpoint(), "https://api.github.com");
    }

    #[test]
    fn test_tracker_endpoint_custom() {
        let mut raw = serde_yaml::Mapping::new();
        let mut tracker = serde_yaml::Mapping::new();
        tracker.insert(
            serde_yaml::Value::String("endpoint".into()),
            serde_yaml::Value::String("https://custom.github.com".into()),
        );
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(tracker),
        );
        assert_eq!(config_with(raw).tracker_endpoint(), "https://custom.github.com");
    }

    #[test]
    fn test_tracker_api_key_plain() {
        let mut raw = serde_yaml::Mapping::new();
        let mut tracker = serde_yaml::Mapping::new();
        tracker.insert(
            serde_yaml::Value::String("api_key".into()),
            serde_yaml::Value::String("secret123".into()),
        );
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(tracker),
        );
        assert_eq!(config_with(raw).tracker_api_key(), Some("secret123".to_string()));
    }

    #[test]
    fn test_tracker_api_key_env_resolution() {
        std::env::set_var("TEST_GH_KEY", "gh_secret");
        let mut raw = serde_yaml::Mapping::new();
        let mut tracker = serde_yaml::Mapping::new();
        tracker.insert(
            serde_yaml::Value::String("api_key".into()),
            serde_yaml::Value::String("$TEST_GH_KEY".into()),
        );
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(tracker),
        );
        assert_eq!(config_with(raw).tracker_api_key(), Some("gh_secret".to_string()));
        std::env::remove_var("TEST_GH_KEY");
    }

    #[test]
    fn test_tracker_api_key_empty_after_resolution() {
        std::env::remove_var("TEST_EMPTY_KEY");
        let mut raw = serde_yaml::Mapping::new();
        let mut tracker = serde_yaml::Mapping::new();
        tracker.insert(
            serde_yaml::Value::String("api_key".into()),
            serde_yaml::Value::String("$TEST_EMPTY_KEY".into()),
        );
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(tracker),
        );
        assert_eq!(config_with(raw).tracker_api_key(), None);
    }

    #[test]
    fn test_tracker_project_slug() {
        let mut raw = serde_yaml::Mapping::new();
        let mut tracker = serde_yaml::Mapping::new();
        tracker.insert(
            serde_yaml::Value::String("project_slug".into()),
            serde_yaml::Value::String("owner/repo".into()),
        );
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(tracker),
        );
        assert_eq!(config_with(raw).tracker_project_slug(), Some("owner/repo".to_string()));
    }

    #[test]
    fn test_tracker_project_number() {
        let mut raw = serde_yaml::Mapping::new();
        let mut tracker = serde_yaml::Mapping::new();
        tracker.insert(
            serde_yaml::Value::String("project_number".into()),
            serde_yaml::Value::Number(42.into()),
        );
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(tracker),
        );
        assert_eq!(config_with(raw).tracker_project_number(), Some(42));
    }

    #[test]
    fn test_active_states_default() {
        let states = empty_config().active_states();
        assert_eq!(states, vec!["todo", "in progress"]);
    }

    #[test]
    fn test_active_states_custom() {
        let mut raw = serde_yaml::Mapping::new();
        let mut tracker = serde_yaml::Mapping::new();
        let seq = vec![
            serde_yaml::Value::String("Backlog".into()),
            serde_yaml::Value::String("Ready".into()),
        ];
        tracker.insert(
            serde_yaml::Value::String("active_states".into()),
            serde_yaml::Value::Sequence(seq),
        );
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(tracker),
        );
        assert_eq!(config_with(raw).active_states(), vec!["backlog", "ready"]);
    }

    #[test]
    fn test_terminal_states_default() {
        let states = empty_config().terminal_states();
        assert_eq!(states, vec!["closed", "cancelled", "canceled", "duplicate", "done"]);
    }

    #[test]
    fn test_poll_interval_default() {
        assert_eq!(empty_config().poll_interval_ms(), 30000);
    }

    #[test]
    fn test_poll_interval_custom() {
        let mut raw = serde_yaml::Mapping::new();
        let mut polling = serde_yaml::Mapping::new();
        polling.insert(
            serde_yaml::Value::String("interval_ms".into()),
            serde_yaml::Value::Number(5000.into()),
        );
        raw.insert(
            serde_yaml::Value::String("polling".into()),
            serde_yaml::Value::Mapping(polling),
        );
        assert_eq!(config_with(raw).poll_interval_ms(), 5000);
    }

    #[test]
    fn test_poll_interval_min_clamp() {
        let mut raw = serde_yaml::Mapping::new();
        let mut polling = serde_yaml::Mapping::new();
        polling.insert(
            serde_yaml::Value::String("interval_ms".into()),
            serde_yaml::Value::Number(100.into()),
        );
        raw.insert(
            serde_yaml::Value::String("polling".into()),
            serde_yaml::Value::Mapping(polling),
        );
        assert_eq!(config_with(raw).poll_interval_ms(), 1000);
    }

    #[test]
    fn test_workspace_root_default() {
        let root = empty_config().workspace_root().unwrap();
        assert!(root.to_string_lossy().contains("sympheo_workspaces"));
    }

    #[test]
    fn test_hook_script() {
        let mut raw = serde_yaml::Mapping::new();
        let mut hooks = serde_yaml::Mapping::new();
        hooks.insert(
            serde_yaml::Value::String("after_create".into()),
            serde_yaml::Value::String("echo hello".into()),
        );
        raw.insert(
            serde_yaml::Value::String("hooks".into()),
            serde_yaml::Value::Mapping(hooks),
        );
        let config = config_with(raw);
        assert_eq!(config.hook_script("after_create"), Some("echo hello".to_string()));
        assert_eq!(config.hook_script("before_run"), None);
    }

    #[test]
    fn test_hook_timeout_default() {
        assert_eq!(empty_config().hook_timeout_ms(), 60000);
    }

    #[test]
    fn test_max_concurrent_agents_default() {
        assert_eq!(empty_config().max_concurrent_agents(), 10);
    }

    #[test]
    fn test_max_concurrent_agents_min_clamp() {
        let mut raw = serde_yaml::Mapping::new();
        let mut agent = serde_yaml::Mapping::new();
        agent.insert(
            serde_yaml::Value::String("max_concurrent_agents".into()),
            serde_yaml::Value::Number(0.into()),
        );
        raw.insert(
            serde_yaml::Value::String("agent".into()),
            serde_yaml::Value::Mapping(agent),
        );
        assert_eq!(config_with(raw).max_concurrent_agents(), 1);
    }

    #[test]
    fn test_max_turns_default() {
        assert_eq!(empty_config().max_turns(), 20);
    }

    #[test]
    fn test_max_turns_min_clamp() {
        let mut raw = serde_yaml::Mapping::new();
        let mut agent = serde_yaml::Mapping::new();
        agent.insert(
            serde_yaml::Value::String("max_turns".into()),
            serde_yaml::Value::Number(0.into()),
        );
        raw.insert(
            serde_yaml::Value::String("agent".into()),
            serde_yaml::Value::Mapping(agent),
        );
        assert_eq!(config_with(raw).max_turns(), 1);
    }

    #[test]
    fn test_max_retry_backoff_default() {
        assert_eq!(empty_config().max_retry_backoff_ms(), 300000);
    }

    #[test]
    fn test_max_retry_backoff_min_clamp() {
        let mut raw = serde_yaml::Mapping::new();
        let mut agent = serde_yaml::Mapping::new();
        agent.insert(
            serde_yaml::Value::String("max_retry_backoff_ms".into()),
            serde_yaml::Value::Number(100.into()),
        );
        raw.insert(
            serde_yaml::Value::String("agent".into()),
            serde_yaml::Value::Mapping(agent),
        );
        assert_eq!(config_with(raw).max_retry_backoff_ms(), 1000);
    }

    #[test]
    fn test_max_concurrent_agents_by_state() {
        let mut raw = serde_yaml::Mapping::new();
        let mut agent = serde_yaml::Mapping::new();
        let mut state_map = serde_yaml::Mapping::new();
        state_map.insert(
            serde_yaml::Value::String("todo".into()),
            serde_yaml::Value::Number(3.into()),
        );
        state_map.insert(
            serde_yaml::Value::String("in progress".into()),
            serde_yaml::Value::Number(5.into()),
        );
        state_map.insert(
            serde_yaml::Value::String("invalid".into()),
            serde_yaml::Value::Number(0.into()),
        );
        agent.insert(
            serde_yaml::Value::String("max_concurrent_agents_by_state".into()),
            serde_yaml::Value::Mapping(state_map),
        );
        raw.insert(
            serde_yaml::Value::String("agent".into()),
            serde_yaml::Value::Mapping(agent),
        );
        let map = config_with(raw).max_concurrent_agents_by_state();
        assert_eq!(map.get("todo"), Some(&3usize));
        assert_eq!(map.get("in progress"), Some(&5usize));
        assert!(!map.contains_key("invalid"));
    }

    #[test]
    fn test_codex_command_default() {
        assert_eq!(empty_config().codex_command(), "opencode run");
    }

    #[test]
    fn test_codex_turn_timeout_default() {
        assert_eq!(empty_config().codex_turn_timeout_ms(), 3600000);
    }

    #[test]
    fn test_codex_read_timeout_default() {
        assert_eq!(empty_config().codex_read_timeout_ms(), 5000);
    }

    #[test]
    fn test_codex_stall_timeout_default() {
        assert_eq!(empty_config().codex_stall_timeout_ms(), 300000);
    }

    #[test]
    fn test_daytona_enabled_default() {
        assert!(!empty_config().daytona_enabled());
    }

    #[test]
    fn test_daytona_api_key() {
        std::env::set_var("TEST_DAYTONA_KEY", "dk");
        let mut raw = serde_yaml::Mapping::new();
        let mut daytona = serde_yaml::Mapping::new();
        daytona.insert(
            serde_yaml::Value::String("api_key".into()),
            serde_yaml::Value::String("$TEST_DAYTONA_KEY".into()),
        );
        raw.insert(
            serde_yaml::Value::String("daytona".into()),
            serde_yaml::Value::Mapping(daytona),
        );
        assert_eq!(config_with(raw).daytona_api_key(), Some("dk".to_string()));
        std::env::remove_var("TEST_DAYTONA_KEY");
    }

    #[test]
    fn test_daytona_api_url_default() {
        assert_eq!(empty_config().daytona_api_url(), "https://api.daytona.io");
    }

    #[test]
    fn test_daytona_target_default() {
        assert_eq!(empty_config().daytona_target(), "us");
    }

    #[test]
    fn test_daytona_image() {
        let mut raw = serde_yaml::Mapping::new();
        let mut daytona = serde_yaml::Mapping::new();
        daytona.insert(
            serde_yaml::Value::String("image".into()),
            serde_yaml::Value::String("my-image".into()),
        );
        raw.insert(
            serde_yaml::Value::String("daytona".into()),
            serde_yaml::Value::Mapping(daytona),
        );
        assert_eq!(config_with(raw).daytona_image(), Some("my-image".to_string()));
    }

    #[test]
    fn test_daytona_timeout_default() {
        assert_eq!(empty_config().daytona_timeout_sec(), 3600);
    }

    #[test]
    fn test_daytona_timeout_min_clamp() {
        let mut raw = serde_yaml::Mapping::new();
        let mut daytona = serde_yaml::Mapping::new();
        daytona.insert(
            serde_yaml::Value::String("timeout_sec".into()),
            serde_yaml::Value::Number(10.into()),
        );
        raw.insert(
            serde_yaml::Value::String("daytona".into()),
            serde_yaml::Value::Mapping(daytona),
        );
        assert_eq!(config_with(raw).daytona_timeout_sec(), 30);
    }

    #[test]
    fn test_daytona_env() {
        std::env::set_var("TEST_DAYTONA_ENV", "val");
        let mut raw = serde_yaml::Mapping::new();
        let mut daytona = serde_yaml::Mapping::new();
        let mut env = serde_yaml::Mapping::new();
        env.insert(
            serde_yaml::Value::String("KEY1".into()),
            serde_yaml::Value::String("v1".into()),
        );
        env.insert(
            serde_yaml::Value::String("KEY2".into()),
            serde_yaml::Value::String("$TEST_DAYTONA_ENV".into()),
        );
        daytona.insert(
            serde_yaml::Value::String("env".into()),
            serde_yaml::Value::Mapping(env),
        );
        raw.insert(
            serde_yaml::Value::String("daytona".into()),
            serde_yaml::Value::Mapping(daytona),
        );
        let map = config_with(raw).daytona_env();
        assert_eq!(map.get("KEY1"), Some(&"v1".to_string()));
        assert_eq!(map.get("KEY2"), Some(&"val".to_string()));
        std::env::remove_var("TEST_DAYTONA_ENV");
    }

    #[test]
    fn test_validate_for_dispatch_ok() {
        let mut raw = serde_yaml::Mapping::new();
        let mut tracker = serde_yaml::Mapping::new();
        tracker.insert(
            serde_yaml::Value::String("kind".into()),
            serde_yaml::Value::String("github".into()),
        );
        tracker.insert(
            serde_yaml::Value::String("api_key".into()),
            serde_yaml::Value::String("key".into()),
        );
        tracker.insert(
            serde_yaml::Value::String("project_slug".into()),
            serde_yaml::Value::String("owner/repo".into()),
        );
        tracker.insert(
            serde_yaml::Value::String("project_number".into()),
            serde_yaml::Value::Number(1.into()),
        );
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(tracker),
        );
        let config = config_with(raw);
        assert!(config.validate_for_dispatch().is_ok());
    }

    #[test]
    fn test_validate_missing_tracker_kind() {
        let config = empty_config();
        assert!(matches!(
            config.validate_for_dispatch(),
            Err(SympheoError::InvalidConfiguration(_))
        ));
    }

    #[test]
    fn test_validate_unsupported_tracker_kind() {
        let mut raw = serde_yaml::Mapping::new();
        let mut tracker = serde_yaml::Mapping::new();
        tracker.insert(
            serde_yaml::Value::String("kind".into()),
            serde_yaml::Value::String("linear".into()),
        );
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(tracker),
        );
        let config = config_with(raw);
        assert!(matches!(
            config.validate_for_dispatch(),
            Err(SympheoError::UnsupportedTrackerKind(_))
        ));
    }

    #[test]
    fn test_validate_missing_api_key() {
        let mut raw = serde_yaml::Mapping::new();
        let mut tracker = serde_yaml::Mapping::new();
        tracker.insert(
            serde_yaml::Value::String("kind".into()),
            serde_yaml::Value::String("github".into()),
        );
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(tracker),
        );
        let config = config_with(raw);
        assert!(matches!(
            config.validate_for_dispatch(),
            Err(SympheoError::MissingTrackerApiKey)
        ));
    }

    #[test]
    fn test_validate_missing_project_slug() {
        let mut raw = serde_yaml::Mapping::new();
        let mut tracker = serde_yaml::Mapping::new();
        tracker.insert(
            serde_yaml::Value::String("kind".into()),
            serde_yaml::Value::String("github".into()),
        );
        tracker.insert(
            serde_yaml::Value::String("api_key".into()),
            serde_yaml::Value::String("key".into()),
        );
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(tracker),
        );
        let config = config_with(raw);
        assert!(matches!(
            config.validate_for_dispatch(),
            Err(SympheoError::MissingTrackerProjectSlug)
        ));
    }

    #[test]
    fn test_validate_missing_project_number() {
        let mut raw = serde_yaml::Mapping::new();
        let mut tracker = serde_yaml::Mapping::new();
        tracker.insert(
            serde_yaml::Value::String("kind".into()),
            serde_yaml::Value::String("github".into()),
        );
        tracker.insert(
            serde_yaml::Value::String("api_key".into()),
            serde_yaml::Value::String("key".into()),
        );
        tracker.insert(
            serde_yaml::Value::String("project_slug".into()),
            serde_yaml::Value::String("owner/repo".into()),
        );
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(tracker),
        );
        let config = config_with(raw);
        assert!(matches!(
            config.validate_for_dispatch(),
            Err(SympheoError::InvalidConfiguration(_))
        ));
    }

    #[test]
    fn test_validate_empty_codex_command() {
        let mut raw = serde_yaml::Mapping::new();
        let mut tracker = serde_yaml::Mapping::new();
        tracker.insert(
            serde_yaml::Value::String("kind".into()),
            serde_yaml::Value::String("github".into()),
        );
        tracker.insert(
            serde_yaml::Value::String("api_key".into()),
            serde_yaml::Value::String("key".into()),
        );
        tracker.insert(
            serde_yaml::Value::String("project_slug".into()),
            serde_yaml::Value::String("owner/repo".into()),
        );
        tracker.insert(
            serde_yaml::Value::String("project_number".into()),
            serde_yaml::Value::Number(1.into()),
        );
        let mut codex = serde_yaml::Mapping::new();
        codex.insert(
            serde_yaml::Value::String("command".into()),
            serde_yaml::Value::String("   ".into()),
        );
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(tracker),
        );
        raw.insert(
            serde_yaml::Value::String("codex".into()),
            serde_yaml::Value::Mapping(codex),
        );
        let config = config_with(raw);
        assert!(matches!(
            config.validate_for_dispatch(),
            Err(SympheoError::InvalidConfiguration(_))
        ));
    }

    #[test]
    fn test_validate_daytona_missing_key() {
        let mut raw = serde_yaml::Mapping::new();
        let mut tracker = serde_yaml::Mapping::new();
        tracker.insert(
            serde_yaml::Value::String("kind".into()),
            serde_yaml::Value::String("github".into()),
        );
        tracker.insert(
            serde_yaml::Value::String("api_key".into()),
            serde_yaml::Value::String("key".into()),
        );
        tracker.insert(
            serde_yaml::Value::String("project_slug".into()),
            serde_yaml::Value::String("owner/repo".into()),
        );
        tracker.insert(
            serde_yaml::Value::String("project_number".into()),
            serde_yaml::Value::Number(1.into()),
        );
        let mut daytona = serde_yaml::Mapping::new();
        daytona.insert(
            serde_yaml::Value::String("enabled".into()),
            serde_yaml::Value::Bool(true),
        );
        raw.insert(
            serde_yaml::Value::String("tracker".into()),
            serde_yaml::Value::Mapping(tracker),
        );
        raw.insert(
            serde_yaml::Value::String("daytona".into()),
            serde_yaml::Value::Mapping(daytona),
        );
        let config = config_with(raw);
        assert!(matches!(
            config.validate_for_dispatch(),
            Err(SympheoError::InvalidConfiguration(_))
        ));
    }
}
