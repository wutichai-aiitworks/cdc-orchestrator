use crate::config::AppConfig;
use crate::error::AppError;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

pub async fn run_docker(cfg: &AppConfig, docker_args: &[&str]) -> Result<String, AppError> {
    let mut cmd = Command::new("docker");
    cmd.args(docker_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    run_with_timeout(cmd, cfg.command_timeout_secs).await
}

pub async fn run_docker_stdin(
    cfg: &AppConfig,
    docker_args: &[&str],
    stdin_data: &str,
) -> Result<String, AppError> {
    let mut cmd = Command::new("docker");
    cmd.args(docker_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::Internal(format!("failed to execute command: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_data.as_bytes())
            .await
            .map_err(|e| AppError::Internal(format!("failed to write command stdin: {e}")))?;
        if !stdin_data.ends_with('\n') {
            stdin
                .write_all(b"\n")
                .await
                .map_err(|e| AppError::Internal(format!("failed to write command stdin: {e}")))?;
        }
        drop(stdin);
    }

    let fut = child.wait_with_output();
    let output = timeout(Duration::from_secs(cfg.command_timeout_secs), fut)
        .await
        .map_err(|_| AppError::Internal("command timed out".to_string()))?
        .map_err(|e| AppError::Internal(format!("failed to execute command: {e}")))?;

    decode_output(output)
}

async fn run_with_timeout(mut cmd: Command, timeout_secs: u64) -> Result<String, AppError> {
    let fut = cmd.output();
    let output = timeout(Duration::from_secs(timeout_secs), fut)
        .await
        .map_err(|_| AppError::Internal("command timed out".to_string()))?
        .map_err(|e| AppError::Internal(format!("failed to execute command: {e}")))?;

    decode_output(output)
}

fn decode_output(output: std::process::Output) -> Result<String, AppError> {
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(stdout)
    } else {
        // Some CLIs (notably rpk) print the reason for a failure on stdout, so falling
        // back to it keeps the actual cause instead of an opaque "no stderr" message.
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !stderr.is_empty() {
            return Err(AppError::Upstream(stderr));
        }
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Err(AppError::Upstream(if stdout.is_empty() {
            match output.status.code() {
                Some(code) => format!("command failed with exit code {code} and no output"),
                None => "command failed with no output".to_string(),
            }
        } else {
            stdout
        }))
    }
}
