use crate::error::SympheoError;
use crate::skills::Skill;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub fn load_skill(path: &Path) -> Result<Skill, SympheoError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| SympheoError::Io(format!("failed to read skill at {}: {}", path.display(), e)))?;
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    Ok(Skill { name, content })
}

pub fn load_skills(
    mapping: &crate::skills::SkillMapping,
    base_dir: &Path,
) -> Result<HashMap<String, Skill>, SympheoError> {
    let mut skills = HashMap::new();

    for (state, rel_path) in &mapping.by_state {
        let path = resolve_skill_path(rel_path, base_dir)?;
        match load_skill(&path) {
            Ok(skill) => {
                info!(state = %state, path = %path.display(), "loaded skill");
                skills.insert(state.clone(), skill);
            }
            Err(e) => {
                warn!(state = %state, path = %path.display(), error = %e, "failed to load skill");
                // Don't fail entirely; missing skill for a state just means no extra instructions.
            }
        }
    }

    // Load default skill if configured and not already loaded under a specific state name
    if let Some(ref default_rel) = mapping.default {
        let path = resolve_skill_path(default_rel, base_dir)?;
        if !skills.values().any(|s| s.name == path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()) {
            match load_skill(&path) {
                Ok(skill) => {
                    info!(name = %skill.name, path = %path.display(), "loaded default skill");
                    skills.insert("default".to_string(), skill);
                }
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "failed to load default skill");
                }
            }
        }
    }

    Ok(skills)
}

fn resolve_skill_path(rel_path: &str, base_dir: &Path) -> Result<PathBuf, SympheoError> {
    let path = PathBuf::from(rel_path);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(base_dir.join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_skill_ok() {
        let tmp = std::env::temp_dir().join(format!("skill_test_{}.md", std::process::id()));
        std::fs::write(&tmp, "# Todo Skill\nAnalyze the issue.").unwrap();
        let skill = load_skill(&tmp).unwrap();
        assert_eq!(skill.name, tmp.file_stem().unwrap().to_string_lossy());
        assert!(skill.content.contains("Analyze"));
        std::fs::remove_file(&tmp).unwrap();
    }

    #[test]
    fn test_load_skill_missing() {
        let result = load_skill(Path::new("/nonexistent/skill.md"));
        assert!(matches!(result, Err(SympheoError::Io(_))));
    }

    #[test]
    fn test_load_skills_empty_mapping() {
        let mapping = crate::skills::SkillMapping::default();
        let skills = load_skills(&mapping, Path::new("/tmp")).unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn test_load_skills_with_mapping() {
        let dir = std::env::temp_dir().join(format!("skills_dir_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.md");
        std::fs::write(&todo_path, "Todo skill").unwrap();

        let mut mapping = crate::skills::SkillMapping::default();
        mapping.by_state.insert("todo".into(), todo_path.to_string_lossy().to_string());

        let skills = load_skills(&mapping, &dir).unwrap();
        assert_eq!(skills.len(), 1);
        assert!(skills.contains_key("todo"));
        assert_eq!(skills["todo"].content, "Todo skill");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_resolve_skill_path_relative() {
        let base = Path::new("/project");
        let resolved = resolve_skill_path("skills/todo.md", base).unwrap();
        assert_eq!(resolved, PathBuf::from("/project/skills/todo.md"));
    }

    #[test]
    fn test_resolve_skill_path_absolute() {
        let resolved = resolve_skill_path("/absolute/path.md", Path::new("/project")).unwrap();
        assert_eq!(resolved, PathBuf::from("/absolute/path.md"));
    }
}
