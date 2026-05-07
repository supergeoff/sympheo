use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum SymphonyError {
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
}

impl From<std::io::Error> for SymphonyError {
    fn from(e: std::io::Error) -> Self {
        SymphonyError::Io(e.to_string())
    }
}
