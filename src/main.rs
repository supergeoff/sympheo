use clap::Parser;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use sympheo::config::typed::ServiceConfig;
use sympheo::orchestrator::tick::Orchestrator;
use sympheo::skills::loader::load_skills;
use sympheo::tracker::IssueTracker;
use sympheo::tracker::github::GithubTracker;
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
    let workflow_path = cli
        .workflow_path
        .unwrap_or_else(|| PathBuf::from("WORKFLOW.md"));
    let workflow_dir = workflow_path
        .parent()
        .unwrap_or(Path::new("."))
        .canonicalize()
        .unwrap_or_else(|_| {
            workflow_path
                .parent()
                .unwrap_or(Path::new("."))
                .to_path_buf()
        });

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

    let skill_mapping = config.skill_mapping();
    let skills = load_skills(&skill_mapping, &workflow_dir).unwrap_or_else(|e| {
        warn!(error = %e, "failed to load skills, continuing without");
        std::collections::HashMap::new()
    });

    if let Err(e) = config.validate_for_dispatch() {
        error!(error = %e, "startup validation failed");
        std::process::exit(1);
    }

    // Local backend pre-resolves the agent CLI binary (`cli.command`) to its
    // absolute host path so the worker can invoke it directly under a scrubbed
    // env, without going through a shim. Fail fast at startup if the binary
    // can't be found — otherwise the operator would only see the symptom once
    // a turn starts ("turn failed: turn reported failure").
    //
    // `validate_for_dispatch` above already rejects empty `cli.command`, so
    // `split_whitespace().next()` here is guaranteed to yield a non-empty bin.
    if !config.daytona_enabled() {
        let cmd = config.cli_command();
        let bin = cmd
            .split_whitespace()
            .next()
            .expect("validate_for_dispatch rejects empty cli.command");
        if sympheo::agent::tool_resolver::resolve_tool(bin).is_none() {
            error!(
                tool = %bin,
                "startup validation failed: agent CLI binary not found on host. \
                 Install it (mise, brew, apt, manual) or enable the daytona backend."
            );
            std::process::exit(1);
        }
    }

    info!("startup validation passed");

    // Startup terminal workspace cleanup
    let tracker = Arc::new(GithubTracker::new(&config)?);
    let _terminal_states = config.terminal_states();
    let _active_states = config.active_states();
    let tracker_for_cleanup = tracker.clone();
    let config_for_cleanup = config.clone();

    async fn do_cleanup(
        tracker: &GithubTracker,
        config: &ServiceConfig,
        active_states: &[String],
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let issues = tracker
            .fetch_issues_by_states(&config.terminal_states())
            .await?;
        use sympheo::workspace::manager::WorkspaceManager;
        let wm = WorkspaceManager::new(config)?;
        let runner = sympheo::agent::runner::AgentRunner::new(config);
        let mut cleaned = 0;
        for issue in issues {
            if let Ok(ref r) = runner {
                let ws_path = wm.workspace_path(&issue.identifier);
                if let Err(e) = r.cleanup_workspace(&ws_path).await {
                    warn!(error = %e, issue_identifier = %issue.identifier, "startup cleanup daytona sandbox failed");
                }
            }
            wm.remove_workspace(
                &issue.identifier,
                &issue.id,
                config.hook_script("before_remove").as_deref(),
            )
            .await;
            cleaned += 1;
        }
        // Orphan directory enumeration
        let root = config.workspace_root()?;
        let mut entries = tokio::fs::read_dir(&root).await?;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("SYMPHEO-") {
                let identifier = name_str.strip_prefix("SYMPHEO-").unwrap_or("").to_string();
                if identifier.is_empty() {
                    continue;
                }
                // Check if this issue is active
                if let Ok(issue_states) = tracker
                    .fetch_issue_states_by_ids(std::slice::from_ref(&identifier))
                    .await
                    && let Some(issue) = issue_states.into_iter().next()
                    && active_states
                        .iter()
                        .any(|s| s == &issue.state.to_lowercase())
                {
                    continue;
                }
                let _ = tokio::fs::remove_dir_all(entry.path()).await;
                cleaned += 1;
            }
        }
        Ok(cleaned)
    }

    let active_states_vec = config.active_states();
    match do_cleanup(&tracker, &config, &active_states_vec).await {
        Ok(count) => {
            info!(cleaned = %count, "startup terminal cleanup completed");
        }
        Err(e) => {
            warn!(error = %e, "startup terminal cleanup failed, scheduling background retry");
            let tracker_retry = tracker_for_cleanup.clone();
            let config_retry = config_for_cleanup.clone();
            let active_states_retry = active_states_vec.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(300));
                loop {
                    interval.tick().await;
                    match do_cleanup(&tracker_retry, &config_retry, &active_states_retry).await {
                        Ok(count) => {
                            info!(cleaned = %count, "background terminal cleanup completed");
                            break;
                        }
                        Err(e) => {
                            warn!(error = %e, "background terminal cleanup failed, retrying in 5 minutes");
                        }
                    }
                }
            });
        }
    }

    let git_adapter: Arc<dyn sympheo::git::GitAdapter> =
        Arc::new(sympheo::git::LocalGitAdapter::new());
    let orchestrator = Arc::new(Orchestrator::new(
        config.clone(),
        tracker,
        skills,
        Some(git_adapter.clone()),
    )?);
    let state = orchestrator.state.clone();

    // Install panic hook so any unhandled panic still terminates spawned CLIs
    // (no zombie opencode after a sympheo crash). Grace period is short here
    // because by the time the hook fires, the runtime is dying.
    sympheo::agent::process_registry::install_panic_hook(tokio::time::Duration::from_millis(500));

    // Global graceful shutdown handler: SIGINT (Ctrl-C) or SIGTERM.
    // Iterates the global process registry, sends SIGTERM, waits 3s grace,
    // then SIGKILL. Drains zombies independently of the orchestrator state map
    // so any process spawned by the local backend is reachable.
    tokio::spawn(async move {
        let mut sigterm =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = %e, "failed to install SIGTERM handler");
                    return;
                }
            };
        tokio::select! {
            res = tokio::signal::ctrl_c() => {
                if let Err(e) = res {
                    warn!(error = %e, "ctrl_c handler error");
                    return;
                }
                info!("SIGINT received, terminating child processes");
            }
            _ = sigterm.recv() => {
                info!("SIGTERM received, terminating child processes");
            }
        }
        sympheo::agent::process_registry::terminate_all_async(tokio::time::Duration::from_secs(3))
            .await;
        std::process::exit(0);
    });

    // Optional HTTP server
    let resolved_port = cli.port.or(config.server_port());
    if let Some(port) = resolved_port {
        info!(port = %port, source = if cli.port.is_some() { "cli" } else { "config" }, "starting http server");
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
        let mut watcher =
            match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res
                    && event.kind.is_modify()
                {
                    let _ = tx.try_send(());
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
            let new_skill_mapping = new_config.skill_mapping();
            let new_skills = match load_skills(&new_skill_mapping, &workflow_dir) {
                Ok(skills) => skills,
                Err(e) => {
                    warn!(error = %e, "failed to reload skills, keeping previous");
                    let st = orch_for_watch.state.read().await;
                    st.skills.clone()
                }
            };
            info!("workflow reloaded");
            orch_for_watch.reload_config(new_config, new_skills).await;
        }
    });

    // Main loop
    let mut current_interval_ms = config.poll_interval_ms();
    let mut interval =
        tokio::time::interval(tokio::time::Duration::from_millis(current_interval_ms));
    interval.tick().await; // first tick immediate-ish

    loop {
        let notify = {
            let st = orchestrator.state.read().await;
            st.refresh_notify.clone()
        };
        tokio::select! {
            _ = interval.tick() => {},
            _ = notify.notified() => {
                info!("manual refresh triggered");
            },
        }
        let cfg = orchestrator.config.read().await.clone();
        let new_interval_ms = cfg.poll_interval_ms();
        if new_interval_ms != current_interval_ms {
            tracing::debug!(old = %current_interval_ms, new = %new_interval_ms, "polling interval changed");
            current_interval_ms = new_interval_ms;
            interval =
                tokio::time::interval(tokio::time::Duration::from_millis(current_interval_ms));
        }
        orchestrator.tick().await;
        orchestrator.process_retries().await;
    }
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
        assert_eq!(
            cli.workflow_path,
            Some(PathBuf::from("/path/to/WORKFLOW.md"))
        );
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
