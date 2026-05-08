use crate::error::SympheoError;
use crate::skills::{Skill, SkillMapping};
use std::collections::HashMap;
use std::path::Path;

pub fn load_skill(path: &Path) -> Result<Skill, SympheoError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| SympheoError::Io(format!("failed to read skill file {}: {}", path.display(), e)))?;
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    Ok(Skill { name, content })
}

pub fn load_skills(
    mapping: &SkillMapping,
    base_dir: &Path,
) -> Result<HashMap<String, Skill>, SympheoError> {
    let mut skills = HashMap::new();

    for (state, rel_path) in &mapping.by_state {
        let path = base_dir.join(rel_path);
        match load_skill(&path) {
            Ok(skill) => {
                skills.insert(state.clone(), skill);
            }
            Err(e) => {
                tracing::warn!(state = %state, path = %path.display(), error = %e, "failed to load skill, skipping");
            }
        }
    }

    if let Some(ref default_path) = mapping.default {
        let path = base_dir.join(default_path);
        if !skills.contains_key("default") {
            match load_skill(&path) {
                Ok(skill) => {
                    skills.insert("default".to_string(), skill);
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "failed to load default skill, skipping");
                }
            }
        }
    }

    Ok(skills)
}
