use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use std::os::windows::process::CommandExt as _;

use cdr_remote_agent::commands::{ProcessError, ProcessOutcome};
use tokio::sync::watch;
use tokio::task::JoinHandle;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct PidCleanup {
    path: PathBuf,
    process_id: Option<u32>,
}

impl PidCleanup {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            process_id: None,
        }
    }

    pub fn configure_diagnostics(&self, environment: &mut HashMap<String, String>) {
        environment.insert(
            "CDR_PROCESS_CONTRACT_TRACE_PATH".into(),
            self.path
                .with_extension("trace")
                .to_string_lossy()
                .into_owned(),
        );
        environment.insert(
            "CDR_PROCESS_CONTRACT_CHILD_TRACE_PATH".into(),
            self.path
                .with_extension("childtrace")
                .to_string_lossy()
                .into_owned(),
        );
        if let Ok(case) = std::env::var("CDR_PROCESS_CONTRACT_DIAG_CASE") {
            environment.insert("CDR_PROCESS_CONTRACT_DIAG_CASE".into(), case);
        }
    }

    pub fn diagnostic(&self) -> String {
        format!(
            "pid-file={}; launcher-trace={}; child-entry={}",
            describe_file(&self.path, 128),
            describe_file(&self.path.with_extension("trace"), 4096),
            describe_file(&self.path.with_extension("childtrace"), 128)
        )
    }

    pub async fn wait_ready(&mut self, timeout: Duration) -> Option<u32> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(process_id) = self.observe() {
                return Some(process_id);
            }
            if Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    pub fn observe(&mut self) -> Option<u32> {
        if self.process_id.is_none() {
            self.process_id = read_pid(&self.path);
        }
        self.process_id
    }

    pub fn wait_gone(&mut self, timeout: Duration) -> Result<bool, String> {
        let process_id = self
            .observe()
            .ok_or_else(|| format!("process disappearance unconfirmed: {}", self.diagnostic()))?;
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(false);
            }
            if !pid_exists(process_id, remaining)? {
                return Ok(true);
            }
            std::thread::sleep(Duration::from_millis(20).min(remaining));
        }
    }
}

// Native owned-job custody supplies the fallback. A numeric PID from a file
// must not authorize termination: it can be reused after the descendant exits.

pub async fn settle_failure(
    task: &mut JoinHandle<Result<ProcessOutcome, ProcessError>>,
    cancel: &watch::Sender<bool>,
) -> String {
    let _ = cancel.send(true);
    match tokio::time::timeout(Duration::from_secs(8), &mut *task).await {
        Ok(result) => format!("operation settlement={result:?}; see separate cleanup/reader trace"),
        Err(_) => {
            task.abort();
            let aborted = tokio::time::timeout(Duration::from_millis(500), task).await;
            format!("cleanup/reader settlement UNCONFIRMED; bounded abort observation={aborted:?}")
        }
    }
}

fn read_bytes(path: &Path, limit: u64) -> Result<Vec<u8>, std::io::Error> {
    let mut bytes = Vec::new();
    File::open(path)?.take(limit).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn read_pid(path: &Path) -> Option<u32> {
    let bytes = read_bytes(path, 128).ok()?;
    std::str::from_utf8(&bytes).ok()?.trim().parse().ok()
}

fn describe_file(path: &Path, limit: u64) -> String {
    match read_bytes(path, limit) {
        Ok(bytes) => format!(
            "bounded-bytes={bytes:?}; decoded={:?}",
            std::str::from_utf8(&bytes)
        ),
        Err(error) => format!("read-error kind={:?}: {error}", error.kind()),
    }
}

fn pid_exists(process_id: u32, timeout: Duration) -> Result<bool, String> {
    let code = helper_status(
        Command::new("powershell.exe").args([
            "-NoProfile", "-NonInteractive", "-Command",
            &format!("if (Get-Process -Id {process_id} -ErrorAction SilentlyContinue) {{ exit 0 }} else {{ exit 1 }}"),
        ]),
        timeout,
    )?;
    match code {
        0 => Ok(true),
        1 => Ok(false),
        value => Err(format!(
            "PID observation returned unexpected status {value}"
        )),
    }
}

fn helper_status(command: &mut Command, timeout: Duration) -> Result<i32, String> {
    let mut child = command
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("helper launch failed: {error}"))?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return status
                    .code()
                    .ok_or_else(|| "helper exit code unavailable".into());
            }
            Ok(None) => {}
            Err(error) => {
                return Err(format!(
                    "helper observation failed: {error}; {}",
                    stop_helper(&mut child)
                ));
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "helper observation timeout; {}",
                stop_helper(&mut child)
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn stop_helper(child: &mut Child) -> String {
    let killed = child.kill();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return format!("retained helper termination confirmed: {status}; kill={killed:?}");
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            result => {
                return format!(
                    "retained helper termination UNCONFIRMED: {result:?}; kill={killed:?}"
                );
            }
        }
    }
}

#[test]
fn diagnostic_helper_timeout_terminates_the_retained_helper() {
    let started = Instant::now();
    let error = helper_status(
        Command::new("powershell.exe").args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 30",
        ]),
        Duration::from_millis(100),
    )
    .expect_err("a stalled helper must remain a timeout failure");
    assert!(error.starts_with("helper observation timeout;"), "{error}");
    assert!(error.contains("termination confirmed"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(3));
}
