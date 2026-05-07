use crate::error::SymphonyError;
use crate::tracker::model::WorkflowDefinition;

pub fn parse(content: &str) -> Result<WorkflowDefinition, SymphonyError> {
    let trimmed = content.trim_start();
    if trimmed.starts_with("---") {
        let after_first = &trimmed[3..];
        if let Some(end_idx) = after_first.find("---") {
            let front_matter = &after_first[..end_idx];
            let body = &after_first[end_idx + 3..];
            let yaml_value: serde_yaml::Value = serde_yaml::from_str(front_matter)
                .map_err(|e| SymphonyError::WorkflowParseError(e.to_string()))?;
            let mapping = yaml_value.as_mapping().ok_or(SymphonyError::WorkflowFrontMatterNotAMap)?;
            Ok(WorkflowDefinition {
                config: mapping.clone(),
                prompt_template: body.trim().to_string(),
            })
        } else {
            Err(SymphonyError::WorkflowParseError(
                "unclosed front matter".into(),
            ))
        }
    } else {
        Ok(WorkflowDefinition {
            config: serde_yaml::Mapping::new(),
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
}
