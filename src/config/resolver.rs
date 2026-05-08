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
        let home = home::home_dir()
            .ok_or_else(|| SympheoError::InvalidConfiguration("cannot resolve home dir".into()))?;
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

pub fn get_string(
    mapping: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    mapping
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

pub fn get_i64(mapping: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<i64> {
    mapping.get(key).and_then(|v| v.as_i64())
}

pub fn get_bool(mapping: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<bool> {
    mapping.get(key).and_then(|v| v.as_bool())
}

pub fn get_str_list(
    mapping: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<Vec<String>> {
    mapping.get(key).and_then(|v| v.as_array()).map(|seq| {
        seq.iter()
            .filter_map(|item| item.as_str().map(|s| s.to_string()))
            .collect()
    })
}

pub fn get_string_map(
    mapping: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    mapping.get(key).and_then(|v| v.as_object()).cloned()
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
        unsafe { env::set_var("TEST_RESOLVE_VAR", "resolved_value") };
        assert_eq!(resolve_value("$TEST_RESOLVE_VAR"), "resolved_value");
        unsafe { env::remove_var("TEST_RESOLVE_VAR") };
    }

    #[test]
    fn test_resolve_value_env_var_missing() {
        unsafe { env::remove_var("TEST_MISSING_VAR_12345") };
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
        let mut map = serde_json::Map::<String, serde_json::Value>::new();
        map.insert("key".into(), serde_json::Value::String("value".into()));
        assert_eq!(get_string(&map, "key"), Some("value".to_string()));
    }

    #[test]
    fn test_get_string_missing() {
        let map = serde_json::Map::<String, serde_json::Value>::new();
        assert_eq!(get_string(&map, "missing"), None);
    }

    #[test]
    fn test_get_i64_found() {
        let mut map = serde_json::Map::<String, serde_json::Value>::new();
        map.insert("num".into(), serde_json::Value::Number(42.into()));
        assert_eq!(get_i64(&map, "num"), Some(42));
    }

    #[test]
    fn test_get_str_list_found() {
        let mut map = serde_json::Map::<String, serde_json::Value>::new();
        let seq = vec![
            serde_json::Value::String("a".into()),
            serde_json::Value::String("b".into()),
        ];
        map.insert("items".into(), serde_json::Value::Array(seq));
        assert_eq!(
            get_str_list(&map, "items"),
            Some(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn test_resolve_path_canonicalize_fallback() {
        let path = resolve_path("/nonexistent/path/abc", Path::new("/tmp")).unwrap();
        assert_eq!(path, PathBuf::from("/nonexistent/path/abc"));
    }

    #[test]
    fn test_get_str_list_with_non_strings() {
        let mut map = serde_json::Map::<String, serde_json::Value>::new();
        let seq = vec![
            serde_json::Value::String("a".into()),
            serde_json::Value::Number(42.into()),
            serde_json::Value::String("b".into()),
        ];
        map.insert("items".into(), serde_json::Value::Array(seq));
        assert_eq!(
            get_str_list(&map, "items"),
            Some(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn test_get_string_map_not_a_map() {
        let mut map = serde_json::Map::<String, serde_json::Value>::new();
        map.insert(
            "config".into(),
            serde_json::Value::String("not_a_map".into()),
        );
        assert_eq!(get_string_map(&map, "config"), None);
    }

    #[test]
    fn test_get_string_map_found() {
        let mut inner = serde_json::Map::<String, serde_json::Value>::new();
        inner.insert("x".into(), serde_json::Value::String("y".into()));
        let mut map = serde_json::Map::<String, serde_json::Value>::new();
        map.insert("config".into(), serde_json::Value::Object(inner.clone()));
        assert_eq!(get_string_map(&map, "config"), Some(inner));
    }
}
