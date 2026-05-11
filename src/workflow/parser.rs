use crate::error::SympheoError;
use crate::tracker::model::WorkflowDefinition;

pub fn parse(content: &str) -> Result<WorkflowDefinition, SympheoError> {
    let trimmed = content.trim_start();

    // Opening fence must be `---` on its own line.
    let Some(after_open) = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"))
    else {
        return Ok(WorkflowDefinition {
            config: serde_json::Map::<String, serde_json::Value>::new(),
            prompt_template: content.trim().to_string(),
        });
    };

    // Scan line by line for a closing fence that IS the full line. A naïve
    // `find("---")` would match `---` inside YAML comments or markdown
    // underlines (`----`) and truncate the front matter early.
    let mut offset = 0usize;
    let mut fence: Option<(usize, usize)> = None;
    for line in after_open.split_inclusive('\n') {
        let stripped = line.trim_end_matches(['\n', '\r']);
        if stripped == "---" {
            fence = Some((offset, offset + line.len()));
            break;
        }
        offset += line.len();
    }
    let (close_start, close_end) =
        fence.ok_or_else(|| SympheoError::WorkflowParseError("unclosed front matter".into()))?;

    let front_matter = &after_open[..close_start];
    let body = &after_open[close_end..];

    let yaml_value: serde_json::Value = serde_saphyr::from_str(front_matter)
        .map_err(|e| SympheoError::WorkflowParseError(e.to_string()))?;
    let mapping = yaml_value
        .as_object()
        .ok_or(SympheoError::WorkflowFrontMatterNotAMap)?;
    Ok(WorkflowDefinition {
        config: mapping.clone(),
        prompt_template: body.trim().to_string(),
    })
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

    #[test]
    fn test_parse_triple_dash_inside_comment_is_not_a_fence() {
        let text = "---\n# this file uses `---` as a fence delimiter\ntracker:\n  kind: github\n---\nFallback prompt";
        let wf =
            parse(text).expect("triple-dash inside a YAML comment must not close the front matter");
        assert_eq!(wf.config["tracker"]["kind"], "github");
        assert_eq!(wf.prompt_template, "Fallback prompt");
    }

    #[test]
    fn test_parse_dashed_underline_in_block_scalar_is_not_a_fence() {
        let text = "---\nprompt: |\n  Body\n  ----\n  some text\n---\nFallback";
        let wf = parse(text).expect(
            "markdown ---- underline inside a block scalar must not close the front matter",
        );
        assert!(wf.config["prompt"].as_str().unwrap().contains("Body"));
        assert_eq!(wf.prompt_template, "Fallback");
    }

    #[test]
    fn test_parse_closing_fence_with_crlf() {
        let text = "---\r\ntracker:\n  kind: github\n---\r\nDo work";
        let wf = parse(text).expect("CRLF line endings around fences must work");
        assert_eq!(wf.config["tracker"]["kind"], "github");
        assert_eq!(wf.prompt_template, "Do work");
    }
}
