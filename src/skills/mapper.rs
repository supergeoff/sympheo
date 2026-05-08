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
                    if let (Some(state), Some(path)) = (k.as_str(), v.as_str()) {
                        by_state.insert(state.to_lowercase(), path.to_string());
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
        let key = issue_state.to_lowercase();
        self.by_state
            .get(&key)
            .map(|s| s.as_str())
            .or(self.default.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn empty_config() -> ServiceConfig {
        ServiceConfig::new(serde_yaml::Mapping::new(), PathBuf::from("/tmp"), "".into())
    }

    fn config_with_skills(raw: HashMap<String, serde_yaml::Value>) -> ServiceConfig {
        let mut mapping = serde_yaml::Mapping::new();
        for (k, v) in raw {
            mapping.insert(serde_yaml::Value::String(k), v);
        }
        let mut root = serde_yaml::Mapping::new();
        root.insert(
            serde_yaml::Value::String("skills".into()),
            serde_yaml::Value::Mapping(mapping),
        );
        ServiceConfig::new(root, PathBuf::from("/tmp"), "".into())
    }

    #[test]
    fn test_from_config_empty() {
        let config = empty_config();
        let mapping = SkillMapping::from_config(&config);
        assert!(mapping.by_state.is_empty());
        assert!(mapping.default.is_none());
    }

    #[test]
    fn test_from_config_with_mapping() {
        let mut raw = HashMap::new();
        let mut m = serde_yaml::Mapping::new();
        m.insert(
            serde_yaml::Value::String("todo".into()),
            serde_yaml::Value::String("skills/todo.md".into()),
        );
        m.insert(
            serde_yaml::Value::String("in progress".into()),
            serde_yaml::Value::String("skills/in_progress.md".into()),
        );
        raw.insert("mapping".into(), serde_yaml::Value::Mapping(m));
        raw.insert("default".into(), serde_yaml::Value::String("skills/default.md".into()));

        let config = config_with_skills(raw);
        let mapping = SkillMapping::from_config(&config);
        assert_eq!(mapping.by_state.len(), 2);
        assert_eq!(mapping.by_state.get("todo"), Some(&"skills/todo.md".to_string()));
        assert_eq!(mapping.by_state.get("in progress"), Some(&"skills/in_progress.md".to_string()));
        assert_eq!(mapping.default, Some("skills/default.md".to_string()));
    }

    #[test]
    fn test_resolve_skill_found() {
        let mut mapping = SkillMapping::default();
        mapping.by_state.insert("todo".into(), "skills/todo.md".into());
        assert_eq!(mapping.resolve_skill("Todo"), Some("skills/todo.md"));
        assert_eq!(mapping.resolve_skill("todo"), Some("skills/todo.md"));
    }

    #[test]
    fn test_resolve_skill_fallback_default() {
        let mapping = SkillMapping {
            default: Some("skills/default.md".into()),
            ..Default::default()
        };
        assert_eq!(mapping.resolve_skill("review"), Some("skills/default.md"));
    }

    #[test]
    fn test_resolve_skill_not_found_no_default() {
        let mapping = SkillMapping::default();
        assert_eq!(mapping.resolve_skill("review"), None);
    }
}
