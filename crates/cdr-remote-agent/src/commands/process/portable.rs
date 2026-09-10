use std::collections::HashMap;
use std::hash::BuildHasher;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;
use tokio::sync::watch;

use super::capture::capture;
use super::{ProcessCompletion, ProcessError, ProcessOutcome, wait_for_cancellation};

pub async fn run<S: BuildHasher>(
    executable: &str,
    arguments: &[String],
    cwd: &Path,
    environment: &HashMap<String, String, S>,
    timeout: Duration,
    max_stream_bytes: usize,
    mut cancelled: watch::Receiver<bool>,
) -> Result<ProcessOutcome, ProcessError> {
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .current_dir(cwd)
        .env_clear()
        .envs(environment)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let process_id = child
        .id()
        .ok_or_else(|| std::io::Error::other("child process has no id"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("stdout unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("stderr unavailable"))?;
    let stdout_reader = tokio::spawn(capture(stdout, max_stream_bytes));
    let stderr_reader = tokio::spawn(capture(stderr, max_stream_bytes));
    let timeout = tokio::time::sleep(timeout);
    tokio::pin!(timeout);
    let stop = tokio::select! {
        status = child.wait() => ProcessStop::Exited(status?),
        () = wait_for_cancellation(&mut cancelled) => ProcessStop::Cancelled,
        () = &mut timeout => ProcessStop::TimedOut,
    };
    let (status, completion) = match stop {
        ProcessStop::Exited(status) => (Some(status), ProcessCompletion::Exited),
        ProcessStop::TimedOut => {
            terminate_tree(&mut child).await?;
            (None, ProcessCompletion::TimedOut)
        }
        ProcessStop::Cancelled => {
            terminate_tree(&mut child).await?;
            (None, ProcessCompletion::Cancelled)
        }
    };
    let stdout = stdout_reader
        .await
        .map_err(|error| ProcessError::Reader(error.to_string()))??;
    let stderr = stderr_reader
        .await
        .map_err(|error| ProcessError::Reader(error.to_string()))??;
    Ok(ProcessOutcome {
        process_id,
        exit_code: status.and_then(|value| value.code()),
        stdout_bytes: stdout.bytes_seen(),
        stderr_bytes: stderr.bytes_seen(),
        stdout: stdout.value(),
        stderr: stderr.value(),
        completion,
        stdout_truncated: stdout.truncated(),
        stderr_truncated: stderr.truncated(),
    })
}

enum ProcessStop {
    Exited(std::process::ExitStatus),
    TimedOut,
    Cancelled,
}

async fn terminate_tree(child: &mut tokio::process::Child) -> Result<(), ProcessError> {
    child.kill().await?;
    let _ = child.wait().await?;
    Ok(())
}
