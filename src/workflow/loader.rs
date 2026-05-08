use crate::error::SympheoError;
use crate::tracker::model::WorkflowDefinition;
use std::path::{Path, PathBuf};

pub struct WorkflowLoader {
    path: PathBuf,
}

impl WorkflowLoader {
    pub fn new(path: Option<PathBuf>) -> Self {
        let path = path.unwrap_or_else(|| PathBuf::from("WORKFLOW.md"));
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<WorkflowDefinition, SympheoError> {
        let content =
            std::fs::read_to_string(&self.path).map_err(|e| {
                SympheoError::MissingWorkflowFile(format!("{}: {}", self.path.display(), e))
            })?;
        super::parser::parse(&content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_existing_file() {
        let tmp = std::env::temp_dir().join(format!("wf_test_{}.md", std::process::id()));
        std::fs::write(&tmp, "---\ntracker:\n  kind: github\n---\nDo work").unwrap();
        let loader = WorkflowLoader::new(Some(tmp.clone()));
        let wf = loader.load().unwrap();
        assert_eq!(wf.prompt_template, "Do work");
        assert!(wf.config.contains_key("tracker"));
        std::fs::remove_file(&tmp).unwrap();
    }

    #[test]
    fn test_load_missing_file() {
        let loader = WorkflowLoader::new(Some(PathBuf::from("/nonexistent/workflow.md")));
        let result = loader.load();
        assert!(matches!(result, Err(SympheoError::MissingWorkflowFile(_))));
    }

    #[test]
    fn test_path_accessor() {
        let loader = WorkflowLoader::new(Some(PathBuf::from("/custom/path.md")));
        assert_eq!(loader.path(), Path::new("/custom/path.md"));
    }

    #[test]
    fn test_default_path() {
        let loader = WorkflowLoader::new(None);
        assert_eq!(loader.path(), Path::new("WORKFLOW.md"));
    }
}
