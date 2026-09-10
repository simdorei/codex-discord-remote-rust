mod capture;
#[cfg(not(windows))]
mod portable;
#[cfg(windows)]
mod windows;

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::path::Path;
use std::time::Duration;

use thiserror::Error;
use tokio::sync::watch;

pub use capture::TRUNCATION_MARKER;

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("process arguments are empty")]
    EmptyArguments,
    #[error("max_stream_bytes is too small for bounded diagnostics")]
    LimitTooSmall,
    #[error("process cleanup failed: {0}")]
    Cleanup(String),
    #[error("process reader task failed: {0}")]
    Reader(String),
    #[cfg(windows)]
    #[error(transparent)]
    Native(#[from] cdr_windows_native::NativeError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessOutcome {
    pub process_id: u32,
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub completion: ProcessCompletion,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessCompletion {
    Exited,
    TimedOut,
    Cancelled,
}

pub async fn run_bounded_process<S: BuildHasher>(
    arguments: &[String],
    cwd: &Path,
    environment: &HashMap<String, String, S>,
    timeout: Duration,
    max_stream_bytes: usize,
) -> Result<ProcessOutcome, ProcessError> {
    let (keep_alive, cancelled) = watch::channel(false);
    let outcome = run_bounded_process_cancellable(
        arguments,
        cwd,
        environment,
        timeout,
        max_stream_bytes,
        cancelled,
    )
    .await;
    drop(keep_alive);
    outcome
}

pub async fn run_bounded_process_cancellable<S: BuildHasher>(
    arguments: &[String],
    cwd: &Path,
    environment: &HashMap<String, String, S>,
    timeout: Duration,
    max_stream_bytes: usize,
    cancelled: watch::Receiver<bool>,
) -> Result<ProcessOutcome, ProcessError> {
    let Some((executable, arguments)) = arguments.split_first() else {
        return Err(ProcessError::EmptyArguments);
    };
    if max_stream_bytes < TRUNCATION_MARKER.len() + 2 {
        return Err(ProcessError::LimitTooSmall);
    }
    #[cfg(windows)]
    return windows::run(
        executable,
        arguments,
        cwd,
        environment,
        timeout,
        max_stream_bytes,
        cancelled,
    )
    .await;
    #[cfg(not(windows))]
    return portable::run(
        executable,
        arguments,
        cwd,
        environment,
        timeout,
        max_stream_bytes,
        cancelled,
    )
    .await;
}

pub(super) async fn wait_for_cancellation(cancelled: &mut watch::Receiver<bool>) {
    loop {
        if *cancelled.borrow() {
            return;
        }
        if cancelled.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}
