use crate::error::SymphonyError;
use regex::Regex;
use std::path::{Path, PathBuf};

lazy_static::lazy_static! {
    static ref VAR_RE: Regex = Regex::new(r"^\$(\w+)$").unwrap();
}

pub fn resolve_value(value: &str) -> String {
    if let Some(cap) = VAR_RE.captures(value) {
        let var_name = cap.get(1).unwrap().as_str();
        std::env::var(var_name).unwrap_or_default()
    } else {
        value.to_string()
    }
}

pub fn resolve_path(value: &str, workflow_dir: &Path) -> Result<PathBuf, SymphonyError> {
    let expanded = if value.starts_with('~') {
        let home = home::home_dir().ok_or_else(|| {
            SymphonyError::InvalidConfiguration("cannot resolve home dir".into())
        })?;
        home.join(value[1..].trim_start_matches('/'))
    } else {
        PathBuf::from(value)
    };

    let resolved = if expanded.is_relative() {
        workflow_dir.join(expanded)
    } else {
        expanded
    };

    Ok(resolved.canonicalize().unwrap_or(resolved))
}

pub fn get_string(mapping: &serde_yaml::Mapping, key: &str) -> Option<String> {
    mapping.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

pub fn get_i64(mapping: &serde_yaml::Mapping, key: &str) -> Option<i64> {
    mapping.get(key).and_then(|v| v.as_i64())
}

pub fn get_str_list(mapping: &serde_yaml::Mapping, key: &str) -> Option<Vec<String>> {
    mapping.get(key).and_then(|v| v.as_sequence()).map(|seq| {
        seq.iter()
            .filter_map(|item| item.as_str().map(|s| s.to_string()))
            .collect()
    })
}

pub fn get_string_map(
    mapping: &serde_yaml::Mapping,
    key: &str,
) -> Option<serde_yaml::Mapping> {
    mapping.get(key).and_then(|v| v.as_mapping()).cloned()
}
