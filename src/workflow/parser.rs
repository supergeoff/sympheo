use crate::error::SympheoError;
use crate::tracker::model::WorkflowDefinition;

pub fn parse(content: &str) -> Result<WorkflowDefinition, SympheoError> {
    let trimmed = content.trim_start();
    if let Some(after_first) = trimmed.strip_prefix("---") {
        if let Some(end_idx) = after_first.find("---") {
            let front_matter = &after_first[..end_idx];
            let body = &after_first[end_idx + 3..];
            let yaml_value: serde_json::Value = serde_saphyr::from_str(front_matter)
                .map_err(|e| SympheoError::WorkflowParseError(e.to_string()))?;
            let mapping = yaml_value
                .as_object()
                .ok_or(SympheoError::WorkflowFrontMatterNotAMap)?;
            Ok(WorkflowDefinition {
                config: mapping.clone(),
                prompt_template: body.trim().to_string(),
            })
        } else {
            Err(SympheoError::WorkflowParseError(
                "unclosed front matter".into(),
            ))
        }
    } else {
        Ok(WorkflowDefinition {
            config: serde_json::Map::<String, serde_json::Value>::new(),
            prompt_template: content.trim().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_no_front_matter() {
        let wf = parse("Hello world").unwrap();
        assert!(wf.config.is_empty());
        assert_eq!(wf.prompt_template, "Hello world");
    }

    #[test]
    fn test_parse_with_front_matter() {
        let text = "---\ntracker:\n  kind: github\n---\nDo work";
        let wf = parse(text).unwrap();
        assert!(!wf.config.is_empty());
        assert_eq!(wf.prompt_template, "Do work");
    }

    #[test]
    fn test_parse_empty_front_matter() {
        let text = "---\n---\nJust prompt";
        // Empty YAML front matter parses as Null, not a Mapping
        let result = parse(text);
        assert!(matches!(
            result,
            Err(SympheoError::WorkflowFrontMatterNotAMap)
        ));
    }

    #[test]
    fn test_parse_unclosed_front_matter() {
        let text = "---\ntracker: kind\nDo work";
        let result = parse(text);
        assert!(matches!(result, Err(SympheoError::WorkflowParseError(_))));
    }

    #[test]
    fn test_parse_front_matter_not_a_map() {
        let text = "---\n- item1\n- item2\n---\nDo work";
        let result = parse(text);
        assert!(matches!(
            result,
            Err(SympheoError::WorkflowFrontMatterNotAMap)
        ));
    }

    #[test]
    fn test_parse_invalid_yaml() {
        let text = "---\ntracker: {bad yaml\n---\nDo work";
        let result = parse(text);
        assert!(matches!(result, Err(SympheoError::WorkflowParseError(_))));
    }

    #[test]
    fn test_parse_trim_whitespace() {
        let text = "   ---\ntracker:\n  kind: github\n---\n  Do work  ";
        let wf = parse(text).unwrap();
        assert_eq!(wf.prompt_template, "Do work");
    }
}
