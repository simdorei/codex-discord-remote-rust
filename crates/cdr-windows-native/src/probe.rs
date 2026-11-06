//! Bounded observation commands. Their descendants belong to one retained Job Object.
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crate::{CapturedWindowProcess, NativeError};

pub struct ProbeOutput {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

type Reader = (
    Receiver<Result<Vec<u8>, String>>,
    std::thread::JoinHandle<()>,
);
fn reader(mut file: File, limit: usize) -> Reader {
    let (send, receive) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = (&mut file)
            .take((limit + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("probe pipe: {error}"))
            .and_then(|_| {
                if bytes.len() > limit {
                    Err("probe_output_limit".into())
                } else {
                    Ok(bytes)
                }
            });
        let _ = send.send(result);
    });
    (receive, thread)
}

/// The deadline includes terminating the Job and collecting both bounded readers.
/// A parent exit is never treated as proof that its descendants or pipes have closed.
#[allow(
    clippy::implicit_hasher,
    reason = "matches the existing native launch environment boundary"
)]
pub fn capture_bounded(
    executable: &Path,
    arguments: &[String],
    cwd: &Path,
    environment: &HashMap<String, String>,
    deadline: Instant,
    cleanup_reserve: Duration,
    output_limit: usize,
) -> Result<ProbeOutput, NativeError> {
    let work_end = deadline
        .checked_sub(cleanup_reserve)
        .ok_or_else(|| NativeError::InvalidInput("probe_timeout_before_spawn".into()))?;
    if Instant::now() >= work_end {
        return Err(NativeError::InvalidInput(
            "probe_timeout_before_spawn".into(),
        ));
    }
    let mut child = CapturedWindowProcess::launch(executable, arguments, cwd, environment)?;
    let (stdout, stdout_thread) = reader(
        child
            .take_stdout()
            .ok_or_else(|| NativeError::InvalidInput("probe_stdout_missing".into()))?,
        output_limit,
    );
    let (stderr, stderr_thread) = reader(
        child
            .take_stderr()
            .ok_or_else(|| NativeError::InvalidInput("probe_stderr_missing".into()))?,
        output_limit,
    );
    let mut out = None;
    let mut err = None;
    let result = loop {
        if out.is_none() {
            out = stdout.try_recv().ok();
        }
        if err.is_none() {
            err = stderr.try_recv().ok();
        }
        if let Some(Err(message)) = out
            .as_ref()
            .filter(|value| value.is_err())
            .or_else(|| err.as_ref().filter(|value| value.is_err()))
        {
            break Err(NativeError::InvalidInput(message.clone()));
        }
        match child.try_wait() {
            Ok(Some(code)) => break Ok(code),
            Ok(None) => {}
            Err(error) => break Err(error),
        }
        if Instant::now() >= work_end {
            break Err(NativeError::InvalidInput("probe_timeout".into()));
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    child
        .terminate_tree(deadline.saturating_duration_since(Instant::now()))
        .map_err(|error| NativeError::InvalidInput(format!("cleanup_unconfirmed: {error}")))?;
    let collect = |value: Option<Result<Vec<u8>, String>>,
                   receive: Receiver<Result<Vec<u8>, String>>| {
        value.map_or_else(
            || {
                receive
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .map_err(|_| {
                        NativeError::InvalidInput("cleanup_unconfirmed: probe reader".into())
                    })
            },
            Ok,
        )
    };
    let out = collect(out, stdout)?;
    let err = collect(err, stderr)?;
    for thread in [stdout_thread, stderr_thread] {
        while !thread.is_finished() {
            if Instant::now() >= deadline {
                return Err(NativeError::InvalidInput(
                    "cleanup_unconfirmed: probe reader exit".into(),
                ));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        thread.join().map_err(|_| {
            NativeError::InvalidInput("cleanup_unconfirmed: probe reader panic".into())
        })?;
    }
    let code = result?;
    Ok(ProbeOutput {
        code,
        stdout: out.map_err(NativeError::InvalidInput)?,
        stderr: err.map_err(NativeError::InvalidInput)?,
    })
}
