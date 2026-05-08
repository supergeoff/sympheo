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
