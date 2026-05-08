use clap::Parser;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use sympheo::config::typed::ServiceConfig;
use sympheo::orchestrator::tick::Orchestrator;
use sympheo::tracker::github::GithubTracker;
use sympheo::tracker::IssueTracker;
use sympheo::workflow::loader::WorkflowLoader;
use tracing::{error, info, warn};

#[derive(Parser, Debug)]
#[command(name = "sympheo")]
#[command(about = "Orchestrates coding agents to get project work done")]
struct Cli {
    /// Path to WORKFLOW.md
    workflow_path: Option<PathBuf>,

    /// HTTP server port (optional)
    #[arg(long)]
    port: Option<u16>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_cli_default() {
        let cli = Cli::parse_from(["sympheo"]);
        assert_eq!(cli.workflow_path, None);
        assert_eq!(cli.port, None);
    }

    #[test]
    fn test_cli_with_workflow_path() {
        let cli = Cli::parse_from(["sympheo", "/path/to/WORKFLOW.md"]);
        assert_eq!(cli.workflow_path, Some(PathBuf::from("/path/to/WORKFLOW.md")));
    }

    #[test]
    fn test_cli_with_port() {
        let cli = Cli::parse_from(["sympheo", "--port", "8080"]);
        assert_eq!(cli.port, Some(8080));
    }

    #[test]
    fn test_cli_with_both() {
        let cli = Cli::parse_from(["sympheo", "--port", "9090", "/path/to/wf.md"]);
        assert_eq!(cli.workflow_path, Some(PathBuf::from("/path/to/wf.md")));
        assert_eq!(cli.port, Some(9090));
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(tracing::Level::INFO.into())
                .from_env_lossy(),
        )
        .init();

    let cli = Cli::parse();
    let workflow_path = cli.workflow_path.unwrap_or_else(|| PathBuf::from("WORKFLOW.md"));
    let workflow_dir = workflow_path
        .parent()
        .unwrap_or(Path::new("."))
        .canonicalize()
        .unwrap_or_else(|_| workflow_path.parent().unwrap_or(Path::new(".")).to_path_buf());

    let loader = WorkflowLoader::new(Some(workflow_path.clone()));
    let workflow = loader.load().map_err(|e| {
        error!(error = %e, "failed to load workflow");
        e
    })?;

    let config = ServiceConfig::new(
        workflow.config,
        workflow_dir.clone(),
        workflow.prompt_template,
    );

    if let Err(e) = config.validate_for_dispatch() {
        error!(error = %e, "startup validation failed");
        std::process::exit(1);
    }

    info!("startup validation passed");

    // Startup terminal workspace cleanup
    let tracker = Arc::new(GithubTracker::new(&config)?);
    let terminal_states = config.terminal_states();
    match tracker.fetch_issues_by_states(&terminal_states).await {
        Ok(issues) => {
            use sympheo::workspace::manager::WorkspaceManager;
            let wm = WorkspaceManager::new(&config)?;
            for issue in issues {
                wm.remove_workspace(&issue.identifier, config.hook_script("before_remove").as_deref())
                    .await;
            }
        }
        Err(e) => {
            warn!(error = %e, "startup terminal cleanup failed");
        }
    }

    let orchestrator = Arc::new(Orchestrator::new(config.clone(), tracker)?);
    let state = orchestrator.state.clone();

    // Optional HTTP server
    if let Some(port) = cli.port {
        let state_clone = state.clone();
        tokio::spawn(async move {
            if let Err(e) = sympheo::server::start_server(port, state_clone).await {
                warn!(error = %e, "http server error");
            }
        });
    }

    // Workflow file watcher
    let orch_for_watch = orchestrator.clone();
    let watch_path = workflow_path.clone();
    tokio::spawn(async move {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let mut watcher = match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                if event.kind.is_modify() {
                    let _ = tx.try_send(());
                }
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                warn!(error = %e, "failed to create file watcher");
                return;
            }
        };
        use notify::Watcher;
        let _ = watcher.watch(&watch_path, notify::RecursiveMode::NonRecursive);
        while rx.recv().await.is_some() {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            let new_workflow = match WorkflowLoader::new(Some(watch_path.clone())).load() {
                Ok(w) => w,
                Err(e) => {
                    warn!(error = %e, "failed to reload workflow");
                    continue;
                }
            };
            let new_config = ServiceConfig::new(
                new_workflow.config,
                workflow_dir.clone(),
                new_workflow.prompt_template,
            );
            info!("workflow reloaded");
            orch_for_watch.reload_config(new_config).await;
        }
    });

    // Main loop
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(config.poll_interval_ms()));
    interval.tick().await; // first tick immediate-ish

    loop {
        interval.tick().await;
        let cfg = orchestrator.config.read().await.clone();
        interval = tokio::time::interval(tokio::time::Duration::from_millis(cfg.poll_interval_ms()));
        orchestrator.tick().await;
        orchestrator.process_retries().await;
    }
}
