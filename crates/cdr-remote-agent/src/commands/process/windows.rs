use std::collections::HashMap;
use std::hash::BuildHasher;
use std::path::Path;
use std::time::Duration;

use cdr_windows_native::CapturedWindowProcess;
use tokio::sync::watch;

use super::capture::capture_identified;
use super::{ProcessCompletion, ProcessError, ProcessOutcome, wait_for_cancellation};

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const PROCESS_CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn run<S: BuildHasher>(
    executable: &str,
    arguments: &[String],
    cwd: &Path,
    environment: &HashMap<String, String, S>,
    timeout: Duration,
    max_stream_bytes: usize,
    mut cancelled: watch::Receiver<bool>,
) -> Result<ProcessOutcome, ProcessError> {
    let environment = environment
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<HashMap<_, _>>();
    eprintln!(
        "[process-diag:{:?}] launch-attempt",
        std::thread::current().name()
    );
    let mut process =
        CapturedWindowProcess::launch(Path::new(executable), arguments, cwd, &environment)
            .inspect_err(|error| eprintln!("[process-diag] launch-error={error}"))?;
    let process_id = process.process_id();
    eprintln!("[process-diag:{process_id}] launch-returned");
    let stdout = process
        .take_stdout()
        .ok_or_else(|| ProcessError::Cleanup("captured stdout is unavailable".into()))?;
    let stderr = process
        .take_stderr()
        .ok_or_else(|| ProcessError::Cleanup("captured stderr is unavailable".into()))?;
    let stdout_reader = tokio::spawn(capture_identified(
        tokio::fs::File::from_std(stdout),
        max_stream_bytes,
        format!("{process_id}:stdout"),
    ));
    let stderr_reader = tokio::spawn(capture_identified(
        tokio::fs::File::from_std(stderr),
        max_stream_bytes,
        format!("{process_id}:stderr"),
    ));
    let timeout = tokio::time::sleep(timeout);
    tokio::pin!(timeout);
    let stop = tokio::select! {
        exit_code = wait_for_exit(&process) => ProcessStop::Exited(exit_code?),
        () = wait_for_cancellation(&mut cancelled) => ProcessStop::Cancelled,
        () = &mut timeout => ProcessStop::TimedOut,
    };
    eprintln!("[process-diag:{process_id}] selected-stop={stop:?}");
    close_owned_job(process).await?;
    let (exit_code, completion) = match stop {
        ProcessStop::Exited(exit_code) => (Some(exit_code), ProcessCompletion::Exited),
        ProcessStop::TimedOut => (None, ProcessCompletion::TimedOut),
        ProcessStop::Cancelled => (None, ProcessCompletion::Cancelled),
    };
    let stdout = stdout_reader
        .await
        .map_err(|error| ProcessError::Reader(error.to_string()))??;
    let stderr = stderr_reader
        .await
        .map_err(|error| ProcessError::Reader(error.to_string()))??;
    Ok(ProcessOutcome {
        process_id,
        exit_code,
        stdout_bytes: stdout.bytes_seen(),
        stderr_bytes: stderr.bytes_seen(),
        stdout: stdout.value(),
        stderr: stderr.value(),
        completion,
        stdout_truncated: stdout.truncated(),
        stderr_truncated: stderr.truncated(),
    })
}

#[derive(Debug)]
enum ProcessStop {
    Exited(i32),
    TimedOut,
    Cancelled,
}

async fn wait_for_exit(process: &CapturedWindowProcess) -> Result<i32, ProcessError> {
    loop {
        if let Some(exit_code) = process.try_wait().inspect_err(|error| {
            eprintln!(
                "[process-diag:{}] try-wait-error={error}",
                process.process_id()
            )
        })? {
            eprintln!(
                "[process-diag:{}] actual-root-exit={exit_code}",
                process.process_id()
            );
            return Ok(exit_code);
        }
        tokio::time::sleep(PROCESS_POLL_INTERVAL).await;
    }
}

async fn close_owned_job(mut process: CapturedWindowProcess) -> Result<(), ProcessError> {
    tokio::task::spawn_blocking(move || {
        let process_id = process.process_id();
        eprintln!("[process-diag:{process_id}] owned-cleanup-enter");
        let result = process.terminate_tree(PROCESS_CLEANUP_TIMEOUT);
        eprintln!("[process-diag:{process_id}] owned-cleanup-return={result:?}");
        result
    })
    .await
    .map_err(|error| ProcessError::Cleanup(format!("cleanup worker failed: {error}")))??;
    Ok(())
}
