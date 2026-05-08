use std::path::PathBuf;
use sympheo::config::typed::ServiceConfig;
use sympheo::skills::loader::load_skills;
use sympheo::skills::SkillMapping;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skills")
}

#[test]
fn test_skill_mapping_from_config_empty() {
    let config = ServiceConfig::new(serde_yaml::Mapping::new(), PathBuf::from("/tmp"), "".into());
    let mapping = config.skill_mapping();
    assert!(mapping.by_state.is_empty());
    assert!(mapping.default.is_none());
}

#[test]
fn test_skill_mapping_from_config_with_values() {
    let mut root = serde_yaml::Mapping::new();
    let mut skills = serde_yaml::Mapping::new();
    let mut mapping = serde_yaml::Mapping::new();
    mapping.insert(
        serde_yaml::Value::String("todo".into()),
        serde_yaml::Value::String("skills/todo.md".into()),
    );
    mapping.insert(
        serde_yaml::Value::String("in progress".into()),
        serde_yaml::Value::String("skills/in_progress.md".into()),
    );
    skills.insert(
        serde_yaml::Value::String("mapping".into()),
        serde_yaml::Value::Mapping(mapping),
    );
    skills.insert(
        serde_yaml::Value::String("default".into()),
        serde_yaml::Value::String("skills/default.md".into()),
    );
    root.insert(
        serde_yaml::Value::String("skills".into()),
        serde_yaml::Value::Mapping(skills),
    );

    let config = ServiceConfig::new(root, PathBuf::from("/tmp"), "".into());
    let skill_mapping = config.skill_mapping();
    assert_eq!(skill_mapping.by_state.len(), 2);
    assert_eq!(
        skill_mapping.by_state.get("todo"),
        Some(&"skills/todo.md".to_string())
    );
    assert_eq!(
        skill_mapping.by_state.get("in progress"),
        Some(&"skills/in_progress.md".to_string())
    );
    assert_eq!(skill_mapping.default, Some("skills/default.md".to_string()));
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

#[test]
fn test_load_skills_from_fixtures() {
    let dir = fixture_dir();
    let mut mapping = SkillMapping::default();
    mapping.by_state.insert("todo".into(), dir.join("todo.md").to_string_lossy().to_string());
    mapping
        .by_state
        .insert("in progress".into(), dir.join("in_progress.md").to_string_lossy().to_string());
    mapping.default = Some(dir.join("default.md").to_string_lossy().to_string());

    let skills = load_skills(&mapping, &dir).unwrap();
    assert_eq!(skills.len(), 3);
    assert!(skills.contains_key("todo"));
    assert!(skills.contains_key("in progress"));
    assert!(skills.contains_key("default"));
    assert!(skills["todo"].content.contains("Analyze the issue"));
    assert!(skills["in progress"].content.contains("Implement the solution"));
    assert!(skills["default"].content.contains("Work on the issue"));
}

#[test]
fn test_load_skills_missing_file_graceful() {
    let mut mapping = SkillMapping::default();
    mapping.by_state.insert("missing".into(), "/nonexistent/skill.md".into());

    let skills = load_skills(&mapping, PathBuf::from("/tmp").as_path()).unwrap();
    assert!(skills.is_empty());
}

#[test]
fn test_load_skills_partial_failure() {
    let dir = fixture_dir();
    let mut mapping = SkillMapping::default();
    mapping.by_state.insert("todo".into(), dir.join("todo.md").to_string_lossy().to_string());
    mapping.by_state.insert("missing".into(), "/nonexistent/skill.md".into());

    let skills = load_skills(&mapping, &dir).unwrap();
    assert_eq!(skills.len(), 1);
    assert!(skills.contains_key("todo"));
}
