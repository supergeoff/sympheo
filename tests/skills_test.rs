use std::path::PathBuf;
use sympheo::config::typed::ServiceConfig;
use sympheo::skills::loader::{load_skill, load_skills};
use sympheo::skills::SkillMapping;

#[test]
fn test_load_skill_from_file() {
    let path = PathBuf::from("tests/fixtures/skills/todo.md");
    let skill = load_skill(&path).unwrap();
    assert_eq!(skill.name, "todo");
    assert!(skill.content.contains("Analyze the issue carefully"));
}

#[test]
fn test_load_skills_with_mapping() {
    let mut mapping = SkillMapping::default();
    mapping.by_state.insert(
        "todo".to_string(),
        "tests/fixtures/skills/todo.md".to_string(),
    );
    mapping.by_state.insert(
        "in progress".to_string(),
        "tests/fixtures/skills/in_progress.md".to_string(),
    );

    let skills = load_skills(&mapping, &PathBuf::from(".")).unwrap();
    assert_eq!(skills.len(), 2);
    assert!(skills.contains_key("todo"));
    assert!(skills.contains_key("in progress"));
    assert!(skills["todo"].content.contains("Analyze the issue carefully"));
}

#[test]
fn test_load_skills_missing_file_warns_not_fails() {
    let mut mapping = SkillMapping::default();
    mapping.by_state.insert(
        "missing".to_string(),
        "tests/fixtures/skills/nonexistent.md".to_string(),
    );

    let skills = load_skills(&mapping, &PathBuf::from(".")).unwrap();
    assert!(skills.is_empty());
}

#[test]
fn test_skill_mapping_from_config() {
    let mut raw = serde_yaml::Mapping::new();
    let mut skills = serde_yaml::Mapping::new();
    let mut mapping = serde_yaml::Mapping::new();
    mapping.insert(
        serde_yaml::Value::String("todo".into()),
        serde_yaml::Value::String("./skills/todo.md".into()),
    );
    skills.insert(
        serde_yaml::Value::String("mapping".into()),
        serde_yaml::Value::Mapping(mapping),
    );
    skills.insert(
        serde_yaml::Value::String("default".into()),
        serde_yaml::Value::String("./skills/default.md".into()),
    );
    raw.insert(
        serde_yaml::Value::String("skills".into()),
        serde_yaml::Value::Mapping(skills),
    );

    let config = ServiceConfig::new(raw, PathBuf::from("/tmp"), "".into());
    let skill_mapping = config.skill_mapping();
    assert_eq!(skill_mapping.by_state.len(), 1);
    assert_eq!(skill_mapping.by_state.get("todo"), Some(&"./skills/todo.md".to_string()));
    assert_eq!(skill_mapping.default, Some("./skills/default.md".to_string()));
}

#[test]
fn test_skill_mapping_resolve() {
    let mut mapping = SkillMapping::default();
    mapping.by_state.insert("todo".to_string(), "skills/todo.md".to_string());
    mapping.default = Some("skills/default.md".to_string());

    assert_eq!(mapping.resolve_skill("Todo"), Some("skills/todo.md"));
    assert_eq!(mapping.resolve_skill("todo"), Some("skills/todo.md"));
    assert_eq!(mapping.resolve_skill("in progress"), Some("skills/default.md"));
    assert_eq!(mapping.resolve_skill("unknown"), Some("skills/default.md"));
}

#[test]
fn test_skill_mapping_empty_returns_none() {
    let mapping = SkillMapping::default();
    assert_eq!(mapping.resolve_skill("todo"), None);
}
