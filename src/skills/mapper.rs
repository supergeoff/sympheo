use crate::config::typed::ServiceConfig;
use crate::skills::SkillMapping;
use std::collections::HashMap;

impl SkillMapping {
    pub fn from_config(config: &ServiceConfig) -> Self {
        let mut by_state = HashMap::new();
        let mut default = None;

        if let Some(skills_map) = config.skills() {
            if let Some(mapping) = skills_map.get("mapping").and_then(|v| v.as_mapping()) {
                for (k, v) in mapping {
                    if let (Some(key), Some(val)) = (k.as_str(), v.as_str()) {
                        by_state.insert(key.to_lowercase(), val.to_string());
                    }
                }
            }
            if let Some(d) = skills_map.get("default").and_then(|v| v.as_str()) {
                default = Some(d.to_string());
            }
        }

        Self { by_state, default }
    }

    pub fn resolve_skill(&self, issue_state: &str) -> Option<&str> {
        let state_lc = issue_state.to_lowercase();
        self.by_state
            .get(&state_lc)
            .map(|s| s.as_str())
            .or(self.default.as_deref())
    }
}
