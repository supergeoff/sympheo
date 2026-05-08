pub mod loader;
pub mod mapper;

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, Default)]
pub struct SkillMapping {
    pub by_state: HashMap<String, String>,
    pub default: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_new() {
        let skill = Skill {
            name: "todo".into(),
            content: "Do analysis".into(),
        };
        assert_eq!(skill.name, "todo");
        assert_eq!(skill.content, "Do analysis");
    }

    #[test]
    fn test_skill_mapping_default() {
        let mapping = SkillMapping::default();
        assert!(mapping.by_state.is_empty());
        assert!(mapping.default.is_none());
    }
}
