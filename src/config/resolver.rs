use crate::error::SympheoError;
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

pub fn resolve_path(value: &str, workflow_dir: &Path) -> Result<PathBuf, SympheoError> {
    let expanded = if let Some(stripped) = value.strip_prefix('~') {
        let home = home::home_dir().ok_or_else(|| {
            SympheoError::InvalidConfiguration("cannot resolve home dir".into())
        })?;
        home.join(stripped.trim_start_matches('/'))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_resolve_value_plain() {
        assert_eq!(resolve_value("hello"), "hello");
        assert_eq!(resolve_value("plain_text"), "plain_text");
    }

    #[test]
    fn test_resolve_value_env_var() {
        env::set_var("TEST_RESOLVE_VAR", "resolved_value");
        assert_eq!(resolve_value("$TEST_RESOLVE_VAR"), "resolved_value");
        env::remove_var("TEST_RESOLVE_VAR");
    }

    #[test]
    fn test_resolve_value_env_var_missing() {
        env::remove_var("TEST_MISSING_VAR_12345");
        assert_eq!(resolve_value("$TEST_MISSING_VAR_12345"), "");
    }

    #[test]
    fn test_resolve_path_home() {
        let path = resolve_path("~/workspaces", Path::new("/home")).unwrap();
        assert!(path.to_string_lossy().contains("workspaces"));
        assert!(!path.to_string_lossy().starts_with('~'));
    }

    #[test]
    fn test_resolve_path_relative() {
        let path = resolve_path("workspaces", Path::new("/home/project")).unwrap();
        assert_eq!(path, PathBuf::from("/home/project/workspaces"));
    }

    #[test]
    fn test_get_string_found() {
        let mut map = serde_yaml::Mapping::new();
        map.insert(
            serde_yaml::Value::String("key".into()),
            serde_yaml::Value::String("value".into()),
        );
        assert_eq!(get_string(&map, "key"), Some("value".to_string()));
    }

    #[test]
    fn test_get_string_missing() {
        let map = serde_yaml::Mapping::new();
        assert_eq!(get_string(&map, "missing"), None);
    }

    #[test]
    fn test_get_i64_found() {
        let mut map = serde_yaml::Mapping::new();
        map.insert(
            serde_yaml::Value::String("num".into()),
            serde_yaml::Value::Number(42.into()),
        );
        assert_eq!(get_i64(&map, "num"), Some(42));
    }

    #[test]
    fn test_get_str_list_found() {
        let mut map = serde_yaml::Mapping::new();
        let seq = vec![
            serde_yaml::Value::String("a".into()),
            serde_yaml::Value::String("b".into()),
        ];
        map.insert(
            serde_yaml::Value::String("items".into()),
            serde_yaml::Value::Sequence(seq),
        );
        assert_eq!(get_str_list(&map, "items"), Some(vec!["a".to_string(), "b".to_string()]));
    }

    #[test]
    fn test_resolve_path_canonicalize_fallback() {
        let path = resolve_path("/nonexistent/path/abc", Path::new("/tmp")).unwrap();
        assert_eq!(path, PathBuf::from("/nonexistent/path/abc"));
    }

    #[test]
    fn test_get_str_list_with_non_strings() {
        let mut map = serde_yaml::Mapping::new();
        let seq = vec![
            serde_yaml::Value::String("a".into()),
            serde_yaml::Value::Number(42.into()),
            serde_yaml::Value::String("b".into()),
        ];
        map.insert(
            serde_yaml::Value::String("items".into()),
            serde_yaml::Value::Sequence(seq),
        );
        assert_eq!(get_str_list(&map, "items"), Some(vec!["a".to_string(), "b".to_string()]));
    }

    #[test]
    fn test_get_string_map_not_a_map() {
        let mut map = serde_yaml::Mapping::new();
        map.insert(
            serde_yaml::Value::String("config".into()),
            serde_yaml::Value::String("not_a_map".into()),
        );
        assert_eq!(get_string_map(&map, "config"), None);
    }

    #[test]
    fn test_get_string_map_found() {
        let mut inner = serde_yaml::Mapping::new();
        inner.insert(
            serde_yaml::Value::String("x".into()),
            serde_yaml::Value::String("y".into()),
        );
        let mut map = serde_yaml::Mapping::new();
        map.insert(
            serde_yaml::Value::String("config".into()),
            serde_yaml::Value::Mapping(inner.clone()),
        );
        assert_eq!(get_string_map(&map, "config"), Some(inner));
    }
}
