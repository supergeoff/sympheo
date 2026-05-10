use crate::error::SympheoError;
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{Duration, timeout};

// PRD-v2 §5.2.3 — execute the post-turn verifications declared on a phase.
// Each command runs in `bash -lc <command>` with `cwd = workspace_path`.
// On the first failure (non-zero exit, timeout, or spawn error) returns
// `SympheoError::VerificationFailed` so the orchestrator's retry machinery
// schedules the retry with backoff. The remaining commands are skipped.
pub async fn run_verifications(
    commands: &[String],
    cwd: &Path,
    env: &HashMap<String, String>,
    cmd_timeout: Duration,
) -> Result<(), SympheoError> {
    for cmd in commands {
        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            continue;
        }
        tracing::info!(cmd = %trimmed, cwd = %cwd.display(), "running phase verification");
        let mut command = Command::new("bash");
        command
            .arg("-lc")
            .arg(trimmed)
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            command.env(k, v);
        }
        let mut child = command.spawn().map_err(|e| {
            SympheoError::VerificationFailed(format!("spawn failed for `{trimmed}`: {e}"))
        })?;

        match timeout(cmd_timeout, child.wait()).await {
            Ok(Ok(status)) if status.success() => continue,
            Ok(Ok(status)) => {
                return Err(SympheoError::VerificationFailed(format!(
                    "`{trimmed}` exited with {status}"
                )));
            }
            Ok(Err(e)) => {
                return Err(SympheoError::VerificationFailed(format!(
                    "`{trimmed}` wait error: {e}"
                )));
            }
            Err(_) => {
                let _ = child.kill().await;
                return Err(SympheoError::VerificationFailed(format!(
                    "`{trimmed}` timed out after {:?}",
                    cmd_timeout
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn tmp_dir(suffix: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("sympheo_verif_{}_{}", std::process::id(), suffix));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[tokio::test]
    async fn test_no_commands_is_ok() {
        let cwd = tmp_dir("noop");
        let env = HashMap::new();
        let r = run_verifications(&[], &cwd, &env, Duration::from_secs(5)).await;
        assert!(r.is_ok());
    }

    #[tokio::test]
    async fn test_all_pass() {
        let cwd = tmp_dir("pass");
        let env = HashMap::new();
        let cmds = vec!["true".to_string(), "echo ok".to_string()];
        let r = run_verifications(&cmds, &cwd, &env, Duration::from_secs(5)).await;
        assert!(r.is_ok());
    }

    #[tokio::test]
    async fn test_first_failure_short_circuits() {
        let cwd = tmp_dir("fail");
        let env = HashMap::new();
        let cmds = vec![
            "true".to_string(),
            "exit 7".to_string(),
            "touch should_not_run".to_string(),
        ];
        let r = run_verifications(&cmds, &cwd, &env, Duration::from_secs(5)).await;
        assert!(matches!(r, Err(SympheoError::VerificationFailed(_))));
        assert!(!cwd.join("should_not_run").exists());
    }

    #[tokio::test]
    async fn test_timeout_returns_failure() {
        let cwd = tmp_dir("timeout");
        let env = HashMap::new();
        let cmds = vec!["sleep 5".to_string()];
        let r = run_verifications(&cmds, &cwd, &env, Duration::from_millis(100)).await;
        assert!(
            matches!(r, Err(SympheoError::VerificationFailed(msg)) if msg.contains("timed out"))
        );
    }

    #[tokio::test]
    async fn test_blank_command_skipped() {
        let cwd = tmp_dir("blank");
        let env = HashMap::new();
        let cmds = vec!["".to_string(), "   ".to_string(), "true".to_string()];
        let r = run_verifications(&cmds, &cwd, &env, Duration::from_secs(5)).await;
        assert!(r.is_ok());
    }

    #[tokio::test]
    async fn test_env_passed_through() {
        let cwd = tmp_dir("env");
        let mut env = HashMap::new();
        env.insert("SYMPHEO_PHASE_NAME".into(), "build".into());
        let cmds = vec![r#"test "$SYMPHEO_PHASE_NAME" = "build""#.to_string()];
        let r = run_verifications(&cmds, &cwd, &env, Duration::from_secs(5)).await;
        assert!(r.is_ok());
    }
}
