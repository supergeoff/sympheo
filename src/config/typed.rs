use crate::config::resolver;
use crate::error::SympheoError;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ServiceConfig {
    raw: serde_json::Map<String, serde_json::Value>,
    workflow_dir: PathBuf,
    pub prompt_template: String,
}

impl ServiceConfig {
    pub fn new(
        raw: serde_json::Map<String, serde_json::Value>,
        workflow_dir: PathBuf,
        prompt_template: String,
    ) -> Self {
        Self {
            raw,
            workflow_dir,
            prompt_template,
        }
    }

    pub fn raw(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.raw
    }

    fn tracker(&self) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.raw.get("tracker").and_then(|v| v.as_object())
    }

    fn polling(&self) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.raw.get("polling").and_then(|v| v.as_object())
    }

    fn workspace(&self) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.raw.get("workspace").and_then(|v| v.as_object())
    }

    fn hooks(&self) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.raw.get("hooks").and_then(|v| v.as_object())
    }

    fn agent(&self) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.raw.get("agent").and_then(|v| v.as_object())
    }

    fn cli(&self) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.raw.get("cli").and_then(|v| v.as_object())
    }

    pub fn daytona(&self) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.raw.get("daytona").and_then(|v| v.as_object())
    }

    pub fn skills(&self) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.raw.get("skills").and_then(|v| v.as_object())
    }

    pub fn tracker_kind(&self) -> Option<String> {
        self.tracker().and_then(|m| resolver::get_string(m, "kind"))
    }

    pub fn tracker_endpoint(&self) -> String {
        self.tracker()
            .and_then(|m| resolver::get_string(m, "endpoint"))
            .unwrap_or_else(|| {
                if self.tracker_kind().as_deref() == Some("linear") {
                    "https://api.linear.app/graphql".to_string()
                } else {
                    "https://api.github.com".to_string()
                }
            })
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

    pub fn fetch_blocked_by(&self) -> bool {
        self.tracker()
            .and_then(|m| resolver::get_bool(m, "fetch_blocked_by"))
            .unwrap_or(false)
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

    pub fn workspace_repo_url(&self) -> Option<String> {
        self.workspace()
            .and_then(|m| resolver::get_string(m, "repo_url"))
    }

    pub fn workspace_git_reset_strategy(&self) -> String {
        self.workspace()
            .and_then(|m| resolver::get_string(m, "git_reset_strategy"))
            .unwrap_or_else(|| "stash".to_string())
            .to_lowercase()
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

    pub fn max_turns_per_state(&self) -> std::collections::HashMap<String, u32> {
        self.agent()
            .and_then(|m| resolver::get_string_map(m, "max_turns_per_state"))
            .map(|map| {
                map.iter()
                    .filter_map(|(k, v)| {
                        let key = k.to_lowercase();
                        let val = v.as_i64()? as u32;
                        Some((key, val.max(1)))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn max_retry_backoff_ms(&self) -> u64 {
        self.agent()
            .and_then(|m| resolver::get_i64(m, "max_retry_backoff_ms"))
            .unwrap_or(300000)
            .max(1000) as u64
    }

    pub fn max_retry_attempts(&self) -> u32 {
        self.agent()
            .and_then(|m| resolver::get_i64(m, "max_retry_attempts"))
            .unwrap_or(5)
            .max(1) as u32
    }

    pub fn server_port(&self) -> Option<u16> {
        self.raw
            .get("server")
            .and_then(|v| v.as_object())
            .and_then(|m| resolver::get_i64(m, "port"))
            .map(|v| v.clamp(1, 65535) as u16)
    }

    pub fn max_concurrent_agents_by_state(&self) -> HashMap<String, usize> {
        let mut map = HashMap::new();
        if let Some(agent_map) = self.agent()
            && let Some(state_map) = agent_map
                .get("max_concurrent_agents_by_state")
                .and_then(|v| v.as_object())
        {
            for (k, v) in state_map {
                if let Some(val) = v.as_i64()
                    && val > 0
                {
                    map.insert(k.to_lowercase(), val as usize);
                }
            }
        }
        map
    }

    pub fn cli_command(&self) -> String {
        self.cli()
            .and_then(|m| resolver::get_string(m, "command"))
            .unwrap_or_else(|| "opencode run".to_string())
    }

    pub fn cli_turn_timeout_ms(&self) -> u64 {
        self.cli()
            .and_then(|m| resolver::get_i64(m, "turn_timeout_ms"))
            .unwrap_or(3600000)
            .max(0) as u64
    }

    pub fn cli_read_timeout_ms(&self) -> u64 {
        self.cli()
            .and_then(|m| resolver::get_i64(m, "read_timeout_ms"))
            .unwrap_or(5000)
            .max(0) as u64
    }

    pub fn cli_stall_timeout_ms(&self) -> i64 {
        // SPEC §5.3.6 default: 300000 (5 min). Operators MAY override in WORKFLOW.md.
        self.cli()
            .and_then(|m| resolver::get_i64(m, "stall_timeout_ms"))
            .unwrap_or(300000)
    }

    /// SPEC §5.3.6: `cli.args` — additional arguments appended to `cli.command` for each turn.
    pub fn cli_args(&self) -> Vec<String> {
        self.cli()
            .and_then(|m| m.get("args"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(resolver::resolve_value))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// SPEC §5.3.6: `cli.env` — environment variables added to the subprocess for each turn.
    /// Values support `$VAR_NAME` indirection (§6.1).
    pub fn cli_env(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        if let Some(cli_map) = self.cli()
            && let Some(env_map) = cli_map.get("env").and_then(|v| v.as_object())
        {
            for (k, v) in env_map {
                if let Some(val) = v.as_str() {
                    map.insert(k.to_string(), resolver::resolve_value(val));
                }
            }
        }
        map
    }

    /// SPEC §5.3.6: `cli.options` — adapter-specific opaque options.
    /// Sympheo does not interpret this map; it is forwarded verbatim to the adapter.
    pub fn cli_options(&self) -> serde_json::Value {
        self.cli()
            .and_then(|m| m.get("options"))
            .cloned()
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
    }

    pub fn continuation_prompt(&self) -> String {
        self.agent()
            .and_then(|m| resolver::get_string(m, "continuation_prompt"))
            .unwrap_or_else(|| {
                "Continue working on the current task. Review the conversation history and proceed with the next step.".into()
            })
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
        if let Some(daytona_map) = self.daytona()
            && let Some(env_map) = daytona_map.get("env").and_then(|v| v.as_object())
        {
            for (k, v) in env_map {
                if let Some(val) = v.as_str() {
                    map.insert(k.to_string(), resolver::resolve_value(val));
                }
            }
        }
        map
    }

    pub fn daytona_mode(&self) -> String {
        self.daytona()
            .and_then(|m| resolver::get_string(m, "mode"))
            .unwrap_or_else(|| "oneshot".to_string())
            .to_lowercase()
    }

    pub fn daytona_repo_url(&self) -> Option<String> {
        self.daytona()
            .and_then(|m| resolver::get_string(m, "repo_url"))
            .filter(|s| !s.is_empty())
    }

    pub fn validate_for_dispatch(&self) -> Result<(), SympheoError> {
        let kind = self
            .tracker_kind()
            .ok_or(SympheoError::InvalidConfiguration(
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
        let cmd = self.cli_command();
        if cmd.trim().is_empty() {
            return Err(SympheoError::InvalidConfiguration(
                "cli.command is empty".into(),
            ));
        }
        // SPEC §6.3 + §10.1: resolve a CLI adapter from cli.command's leading binary token.
        // Fails with CliAdapterNotFound if no adapter matches (§5.5).
        let _ = crate::agent::cli::select_adapter(&cmd)?;
        if self.daytona_enabled() && self.daytona_api_key().is_none() {
            return Err(SympheoError::InvalidConfiguration(
                "daytona.api_key is required when backend is enabled".into(),
            ));
        }
        Ok(())
    }

    pub fn skill_mapping(&self) -> crate::skills::SkillMapping {
        crate::skills::SkillMapping::from_config(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn empty_config() -> ServiceConfig {
        ServiceConfig::new(
            serde_json::Map::<String, serde_json::Value>::new(),
            PathBuf::from("/tmp"),
            "".into(),
        )
    }

    fn config_with(raw: serde_json::Map<String, serde_json::Value>) -> ServiceConfig {
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
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert("kind".into(), serde_json::Value::String("github".into()));
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        assert_eq!(config_with(raw).tracker_kind(), Some("github".to_string()));
    }

    #[test]
    fn test_tracker_endpoint_default() {
        assert_eq!(empty_config().tracker_endpoint(), "https://api.github.com");
    }

    #[test]
    fn test_tracker_endpoint_custom() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert(
            "endpoint".into(),
            serde_json::Value::String("https://custom.github.com".into()),
        );
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        assert_eq!(
            config_with(raw).tracker_endpoint(),
            "https://custom.github.com"
        );
    }

    #[test]
    fn test_tracker_api_key_plain() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert(
            "api_key".into(),
            serde_json::Value::String("secret123".into()),
        );
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        assert_eq!(
            config_with(raw).tracker_api_key(),
            Some("secret123".to_string())
        );
    }

    #[test]
    fn test_tracker_api_key_env_resolution() {
        unsafe { std::env::set_var("TEST_GH_KEY", "gh_secret") };
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert(
            "api_key".into(),
            serde_json::Value::String("$TEST_GH_KEY".into()),
        );
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        assert_eq!(
            config_with(raw).tracker_api_key(),
            Some("gh_secret".to_string())
        );
        unsafe { std::env::remove_var("TEST_GH_KEY") };
    }

    #[test]
    fn test_tracker_api_key_empty_after_resolution() {
        unsafe { std::env::remove_var("TEST_EMPTY_KEY") };
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert(
            "api_key".into(),
            serde_json::Value::String("$TEST_EMPTY_KEY".into()),
        );
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        assert_eq!(config_with(raw).tracker_api_key(), None);
    }

    #[test]
    fn test_tracker_project_slug() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert(
            "project_slug".into(),
            serde_json::Value::String("owner/repo".into()),
        );
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        assert_eq!(
            config_with(raw).tracker_project_slug(),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn test_tracker_project_number() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert(
            "project_number".into(),
            serde_json::Value::Number(42.into()),
        );
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        assert_eq!(config_with(raw).tracker_project_number(), Some(42));
    }

    #[test]
    fn test_active_states_default() {
        let states = empty_config().active_states();
        assert_eq!(states, vec!["todo", "in progress"]);
    }

    #[test]
    fn test_active_states_custom() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        let seq = vec![
            serde_json::Value::String("Backlog".into()),
            serde_json::Value::String("Ready".into()),
        ];
        tracker.insert("active_states".into(), serde_json::Value::Array(seq));
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        assert_eq!(config_with(raw).active_states(), vec!["backlog", "ready"]);
    }

    #[test]
    fn test_terminal_states_default() {
        let states = empty_config().terminal_states();
        assert_eq!(
            states,
            vec!["closed", "cancelled", "canceled", "duplicate", "done"]
        );
    }

    #[test]
    fn test_poll_interval_default() {
        assert_eq!(empty_config().poll_interval_ms(), 30000);
    }

    #[test]
    fn test_poll_interval_custom() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut polling = serde_json::Map::<String, serde_json::Value>::new();
        polling.insert("interval_ms".into(), serde_json::Value::Number(5000.into()));
        raw.insert("polling".into(), serde_json::Value::Object(polling));
        assert_eq!(config_with(raw).poll_interval_ms(), 5000);
    }

    #[test]
    fn test_poll_interval_min_clamp() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut polling = serde_json::Map::<String, serde_json::Value>::new();
        polling.insert("interval_ms".into(), serde_json::Value::Number(100.into()));
        raw.insert("polling".into(), serde_json::Value::Object(polling));
        assert_eq!(config_with(raw).poll_interval_ms(), 1000);
    }

    #[test]
    fn test_workspace_root_default() {
        let root = empty_config().workspace_root().unwrap();
        assert!(root.to_string_lossy().contains("sympheo_workspaces"));
    }

    #[test]
    fn test_hook_script() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut hooks = serde_json::Map::<String, serde_json::Value>::new();
        hooks.insert(
            "after_create".into(),
            serde_json::Value::String("echo hello".into()),
        );
        raw.insert("hooks".into(), serde_json::Value::Object(hooks));
        let config = config_with(raw);
        assert_eq!(
            config.hook_script("after_create"),
            Some("echo hello".to_string())
        );
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
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut agent = serde_json::Map::<String, serde_json::Value>::new();
        agent.insert(
            "max_concurrent_agents".into(),
            serde_json::Value::Number(0.into()),
        );
        raw.insert("agent".into(), serde_json::Value::Object(agent));
        assert_eq!(config_with(raw).max_concurrent_agents(), 1);
    }

    #[test]
    fn test_max_turns_default() {
        assert_eq!(empty_config().max_turns(), 20);
    }

    #[test]
    fn test_max_turns_min_clamp() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut agent = serde_json::Map::<String, serde_json::Value>::new();
        agent.insert("max_turns".into(), serde_json::Value::Number(0.into()));
        raw.insert("agent".into(), serde_json::Value::Object(agent));
        assert_eq!(config_with(raw).max_turns(), 1);
    }

    #[test]
    fn test_max_retry_backoff_default() {
        assert_eq!(empty_config().max_retry_backoff_ms(), 300000);
    }

    #[test]
    fn test_max_retry_backoff_min_clamp() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut agent = serde_json::Map::<String, serde_json::Value>::new();
        agent.insert(
            "max_retry_backoff_ms".into(),
            serde_json::Value::Number(100.into()),
        );
        raw.insert("agent".into(), serde_json::Value::Object(agent));
        assert_eq!(config_with(raw).max_retry_backoff_ms(), 1000);
    }

    #[test]
    fn test_max_concurrent_agents_by_state() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut agent = serde_json::Map::<String, serde_json::Value>::new();
        let mut state_map = serde_json::Map::<String, serde_json::Value>::new();
        state_map.insert("todo".into(), serde_json::Value::Number(3.into()));
        state_map.insert("in progress".into(), serde_json::Value::Number(5.into()));
        state_map.insert("invalid".into(), serde_json::Value::Number(0.into()));
        agent.insert(
            "max_concurrent_agents_by_state".into(),
            serde_json::Value::Object(state_map),
        );
        raw.insert("agent".into(), serde_json::Value::Object(agent));
        let map = config_with(raw).max_concurrent_agents_by_state();
        assert_eq!(map.get("todo"), Some(&3usize));
        assert_eq!(map.get("in progress"), Some(&5usize));
        assert!(!map.contains_key("invalid"));
    }

    #[test]
    fn test_cli_command_default() {
        assert_eq!(empty_config().cli_command(), "opencode run");
    }

    #[test]
    fn test_continuation_prompt_default() {
        assert!(
            empty_config()
                .continuation_prompt()
                .contains("Continue working")
        );
    }

    #[test]
    fn test_continuation_prompt_custom() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut agent = serde_json::Map::<String, serde_json::Value>::new();
        agent.insert(
            "continuation_prompt".into(),
            serde_json::Value::String("Continuez le travail".into()),
        );
        raw.insert("agent".into(), serde_json::Value::Object(agent));
        assert_eq!(
            config_with(raw).continuation_prompt(),
            "Continuez le travail"
        );
    }

    #[test]
    fn test_tracker_endpoint_default_github() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert("kind".into(), serde_json::Value::String("github".into()));
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        assert_eq!(
            config_with(raw).tracker_endpoint(),
            "https://api.github.com"
        );
    }

    #[test]
    fn test_tracker_endpoint_default_linear() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert("kind".into(), serde_json::Value::String("linear".into()));
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        assert_eq!(
            config_with(raw).tracker_endpoint(),
            "https://api.linear.app/graphql"
        );
    }

    #[test]
    fn test_cli_turn_timeout_default() {
        assert_eq!(empty_config().cli_turn_timeout_ms(), 3600000);
    }

    #[test]
    fn test_cli_read_timeout_default() {
        assert_eq!(empty_config().cli_read_timeout_ms(), 5000);
    }

    #[test]
    fn test_cli_stall_timeout_default() {
        // SPEC §5.3.6 default: 300000 (5 min)
        assert_eq!(empty_config().cli_stall_timeout_ms(), 300000);
    }

    #[test]
    fn test_cli_args_default_empty() {
        assert!(empty_config().cli_args().is_empty());
    }

    #[test]
    fn test_cli_args_custom() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        cli.insert(
            "args".into(),
            serde_json::Value::Array(vec![
                serde_json::Value::String("--verbose".into()),
                serde_json::Value::String("--model=claude".into()),
            ]),
        );
        raw.insert("cli".into(), serde_json::Value::Object(cli));
        assert_eq!(
            config_with(raw).cli_args(),
            vec!["--verbose".to_string(), "--model=claude".to_string()]
        );
    }

    #[test]
    fn test_cli_args_env_resolution() {
        unsafe { std::env::set_var("TEST_CLI_FLAG", "--strict") };
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        cli.insert(
            "args".into(),
            serde_json::Value::Array(vec![serde_json::Value::String("$TEST_CLI_FLAG".into())]),
        );
        raw.insert("cli".into(), serde_json::Value::Object(cli));
        assert_eq!(config_with(raw).cli_args(), vec!["--strict".to_string()]);
        unsafe { std::env::remove_var("TEST_CLI_FLAG") };
    }

    #[test]
    fn test_cli_env_default_empty() {
        assert!(empty_config().cli_env().is_empty());
    }

    #[test]
    fn test_cli_env_resolution() {
        unsafe { std::env::set_var("TEST_CLI_ENV_VAL", "resolved") };
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        let mut env = serde_json::Map::<String, serde_json::Value>::new();
        env.insert(
            "MODEL".into(),
            serde_json::Value::String("$TEST_CLI_ENV_VAL".into()),
        );
        env.insert(
            "STATIC".into(),
            serde_json::Value::String("static-value".into()),
        );
        cli.insert("env".into(), serde_json::Value::Object(env));
        raw.insert("cli".into(), serde_json::Value::Object(cli));
        let env_map = config_with(raw).cli_env();
        assert_eq!(env_map.get("MODEL"), Some(&"resolved".to_string()));
        assert_eq!(env_map.get("STATIC"), Some(&"static-value".to_string()));
        unsafe { std::env::remove_var("TEST_CLI_ENV_VAL") };
    }

    #[test]
    fn test_cli_options_default_empty_object() {
        let opts = empty_config().cli_options();
        assert!(opts.is_object());
        assert!(opts.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_cli_options_passthrough() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        let mut opts = serde_json::Map::<String, serde_json::Value>::new();
        opts.insert("model".into(), serde_json::Value::String("opus".into()));
        opts.insert("permissions".into(), serde_json::Value::Bool(true));
        cli.insert("options".into(), serde_json::Value::Object(opts));
        raw.insert("cli".into(), serde_json::Value::Object(cli));
        let result = config_with(raw).cli_options();
        assert_eq!(result.get("model").and_then(|v| v.as_str()), Some("opus"));
        assert_eq!(
            result.get("permissions").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn test_daytona_enabled_default() {
        assert!(!empty_config().daytona_enabled());
    }

    #[test]
    fn test_daytona_api_key() {
        unsafe { std::env::set_var("TEST_DAYTONA_KEY", "dk") };
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut daytona = serde_json::Map::<String, serde_json::Value>::new();
        daytona.insert(
            "api_key".into(),
            serde_json::Value::String("$TEST_DAYTONA_KEY".into()),
        );
        raw.insert("daytona".into(), serde_json::Value::Object(daytona));
        assert_eq!(config_with(raw).daytona_api_key(), Some("dk".to_string()));
        unsafe { std::env::remove_var("TEST_DAYTONA_KEY") };
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
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut daytona = serde_json::Map::<String, serde_json::Value>::new();
        daytona.insert("image".into(), serde_json::Value::String("my-image".into()));
        raw.insert("daytona".into(), serde_json::Value::Object(daytona));
        assert_eq!(
            config_with(raw).daytona_image(),
            Some("my-image".to_string())
        );
    }

    #[test]
    fn test_daytona_timeout_default() {
        assert_eq!(empty_config().daytona_timeout_sec(), 3600);
    }

    #[test]
    fn test_daytona_timeout_min_clamp() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut daytona = serde_json::Map::<String, serde_json::Value>::new();
        daytona.insert("timeout_sec".into(), serde_json::Value::Number(10.into()));
        raw.insert("daytona".into(), serde_json::Value::Object(daytona));
        assert_eq!(config_with(raw).daytona_timeout_sec(), 30);
    }

    #[test]
    fn test_daytona_env() {
        unsafe { std::env::set_var("TEST_DAYTONA_ENV", "val") };
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut daytona = serde_json::Map::<String, serde_json::Value>::new();
        let mut env = serde_json::Map::<String, serde_json::Value>::new();
        env.insert("KEY1".into(), serde_json::Value::String("v1".into()));
        env.insert(
            "KEY2".into(),
            serde_json::Value::String("$TEST_DAYTONA_ENV".into()),
        );
        daytona.insert("env".into(), serde_json::Value::Object(env));
        raw.insert("daytona".into(), serde_json::Value::Object(daytona));
        let map = config_with(raw).daytona_env();
        assert_eq!(map.get("KEY1"), Some(&"v1".to_string()));
        assert_eq!(map.get("KEY2"), Some(&"val".to_string()));
        unsafe { std::env::remove_var("TEST_DAYTONA_ENV") };
    }

    #[test]
    fn test_validate_for_dispatch_ok() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert("kind".into(), serde_json::Value::String("github".into()));
        tracker.insert("api_key".into(), serde_json::Value::String("key".into()));
        tracker.insert(
            "project_slug".into(),
            serde_json::Value::String("owner/repo".into()),
        );
        tracker.insert("project_number".into(), serde_json::Value::Number(1.into()));
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
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
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert("kind".into(), serde_json::Value::String("linear".into()));
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        let config = config_with(raw);
        assert!(matches!(
            config.validate_for_dispatch(),
            Err(SympheoError::UnsupportedTrackerKind(_))
        ));
    }

    #[test]
    fn test_validate_missing_api_key() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert("kind".into(), serde_json::Value::String("github".into()));
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        let config = config_with(raw);
        assert!(matches!(
            config.validate_for_dispatch(),
            Err(SympheoError::MissingTrackerApiKey)
        ));
    }

    #[test]
    fn test_validate_missing_project_slug() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert("kind".into(), serde_json::Value::String("github".into()));
        tracker.insert("api_key".into(), serde_json::Value::String("key".into()));
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        let config = config_with(raw);
        assert!(matches!(
            config.validate_for_dispatch(),
            Err(SympheoError::MissingTrackerProjectSlug)
        ));
    }

    #[test]
    fn test_validate_missing_project_number() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert("kind".into(), serde_json::Value::String("github".into()));
        tracker.insert("api_key".into(), serde_json::Value::String("key".into()));
        tracker.insert(
            "project_slug".into(),
            serde_json::Value::String("owner/repo".into()),
        );
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        let config = config_with(raw);
        assert!(matches!(
            config.validate_for_dispatch(),
            Err(SympheoError::InvalidConfiguration(_))
        ));
    }

    #[test]
    fn test_validate_empty_cli_command() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert("kind".into(), serde_json::Value::String("github".into()));
        tracker.insert("api_key".into(), serde_json::Value::String("key".into()));
        tracker.insert(
            "project_slug".into(),
            serde_json::Value::String("owner/repo".into()),
        );
        tracker.insert("project_number".into(), serde_json::Value::Number(1.into()));
        let mut cli = serde_json::Map::<String, serde_json::Value>::new();
        cli.insert("command".into(), serde_json::Value::String("   ".into()));
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        raw.insert("cli".into(), serde_json::Value::Object(cli));
        let config = config_with(raw);
        assert!(matches!(
            config.validate_for_dispatch(),
            Err(SympheoError::InvalidConfiguration(_))
        ));
    }

    #[test]
    fn test_validate_daytona_missing_key() {
        let mut raw = serde_json::Map::<String, serde_json::Value>::new();
        let mut tracker = serde_json::Map::<String, serde_json::Value>::new();
        tracker.insert("kind".into(), serde_json::Value::String("github".into()));
        tracker.insert("api_key".into(), serde_json::Value::String("key".into()));
        tracker.insert(
            "project_slug".into(),
            serde_json::Value::String("owner/repo".into()),
        );
        tracker.insert("project_number".into(), serde_json::Value::Number(1.into()));
        let mut daytona = serde_json::Map::<String, serde_json::Value>::new();
        daytona.insert("enabled".into(), serde_json::Value::Bool(true));
        raw.insert("tracker".into(), serde_json::Value::Object(tracker));
        raw.insert("daytona".into(), serde_json::Value::Object(daytona));
        let config = config_with(raw);
        assert!(matches!(
            config.validate_for_dispatch(),
            Err(SympheoError::InvalidConfiguration(_))
        ));
    }
}
