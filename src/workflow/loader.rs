use crate::error::SymphonyError;
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

    pub fn load(&self) -> Result<WorkflowDefinition, SymphonyError> {
        let content =
            std::fs::read_to_string(&self.path).map_err(|e| {
                SymphonyError::MissingWorkflowFile(format!("{}: {}", self.path.display(), e))
            })?;
        super::parser::parse(&content)
    }
}
