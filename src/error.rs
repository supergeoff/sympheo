use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum SympheoError {
    #[error("missing workflow file: {0}")]
    MissingWorkflowFile(String),

    #[error("workflow parse error: {0}")]
    WorkflowParseError(String),

    #[error("workflow front matter is not a map")]
    WorkflowFrontMatterNotAMap,

    #[error("template parse error: {0}")]
    TemplateParseError(String),

    #[error("template render error: {0}")]
    TemplateRenderError(String),

    #[error("unsupported tracker kind: {0}")]
    UnsupportedTrackerKind(String),

    #[error("missing tracker api key")]
    MissingTrackerApiKey,

    #[error("missing tracker project slug")]
    MissingTrackerProjectSlug,

    #[error("tracker api request failed: {0}")]
    TrackerApiRequest(String),

    #[error("tracker api status: {0}")]
    TrackerApiStatus(String),

    #[error("tracker api returned malformed payload: {0}")]
    TrackerMalformedPayload(String),

    #[error("workspace error: {0}")]
    WorkspaceError(String),

    #[error("hook failed: {0}")]
    HookFailed(String),

    #[error("agent runner error: {0}")]
    AgentRunnerError(String),

    #[error("agent process exited unexpectedly")]
    AgentProcessExit,

    #[error("agent turn timeout")]
    AgentTurnTimeout,

    #[error("agent stall detected")]
    AgentStallDetected,

    #[error("invalid configuration: {0}")]
    InvalidConfiguration(String),

    #[error("io error: {0}")]
    Io(String),

    #[error("daytona api error: {0}")]
    DaytonaApiError(String),

    #[error("git error: {0}")]
    GitError(String),
}

impl From<std::io::Error> for SympheoError {
    fn from(e: std::io::Error) -> Self {
        SympheoError::Io(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_variants() {
        assert_eq!(
            format!("{}", SympheoError::MissingWorkflowFile("path".into())),
            "missing workflow file: path"
        );
        assert_eq!(
            format!("{}", SympheoError::WorkflowParseError("bad".into())),
            "workflow parse error: bad"
        );
        assert_eq!(
            format!("{}", SympheoError::WorkflowFrontMatterNotAMap),
            "workflow front matter is not a map"
        );
        assert_eq!(
            format!("{}", SympheoError::TemplateParseError("tpl".into())),
            "template parse error: tpl"
        );
        assert_eq!(
            format!("{}", SympheoError::TemplateRenderError("rend".into())),
            "template render error: rend"
        );
        assert_eq!(
            format!("{}", SympheoError::UnsupportedTrackerKind("linear".into())),
            "unsupported tracker kind: linear"
        );
        assert_eq!(
            format!("{}", SympheoError::MissingTrackerApiKey),
            "missing tracker api key"
        );
        assert_eq!(
            format!("{}", SympheoError::MissingTrackerProjectSlug),
            "missing tracker project slug"
        );
        assert_eq!(
            format!("{}", SympheoError::TrackerApiRequest("fail".into())),
            "tracker api request failed: fail"
        );
        assert_eq!(
            format!("{}", SympheoError::TrackerApiStatus("404".into())),
            "tracker api status: 404"
        );
        assert_eq!(
            format!("{}", SympheoError::TrackerMalformedPayload("bad".into())),
            "tracker api returned malformed payload: bad"
        );
        assert_eq!(
            format!("{}", SympheoError::WorkspaceError("err".into())),
            "workspace error: err"
        );
        assert_eq!(
            format!("{}", SympheoError::HookFailed("hook".into())),
            "hook failed: hook"
        );
        assert_eq!(
            format!("{}", SympheoError::AgentRunnerError("run".into())),
            "agent runner error: run"
        );
        assert_eq!(
            format!("{}", SympheoError::AgentProcessExit),
            "agent process exited unexpectedly"
        );
        assert_eq!(
            format!("{}", SympheoError::AgentTurnTimeout),
            "agent turn timeout"
        );
        assert_eq!(
            format!("{}", SympheoError::AgentStallDetected),
            "agent stall detected"
        );
        assert_eq!(
            format!("{}", SympheoError::InvalidConfiguration("cfg".into())),
            "invalid configuration: cfg"
        );
        assert_eq!(
            format!("{}", SympheoError::Io("io".into())),
            "io error: io"
        );
        assert_eq!(
            format!("{}", SympheoError::DaytonaApiError("api".into())),
            "daytona api error: api"
        );
        assert_eq!(
            format!("{}", SympheoError::GitError("merge conflict".into())),
            "git error: merge conflict"
        );
    }

    #[test]
    fn test_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err: SympheoError = io_err.into();
        assert!(matches!(err, SympheoError::Io(_)));
        assert!(format!("{err}").contains("file missing"));
    }
}
