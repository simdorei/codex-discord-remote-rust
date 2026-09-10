use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use std::os::windows::process::CommandExt as _;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct PidCleanup {
    path: PathBuf,
    process_id: Option<u32>,
    armed: bool,
}

impl PidCleanup {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            process_id: None,
            armed: true,
        }
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
        let Some(process_id) = self.observe() else {
            return Ok(true);
        };
        let deadline = Instant::now() + timeout;
        loop {
            if !pid_exists(process_id)? {
                self.armed = false;
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for PidCleanup {
    fn drop(&mut self) {
        if self.armed
            && let Some(process_id) = self.observe()
        {
            stop_pid(process_id);
        }
    }
}

fn read_pid(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn pid_exists(process_id: u32) -> Result<bool, String> {
    let status = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "if (Get-Process -Id {process_id} -ErrorAction SilentlyContinue) {{ exit 0 }} else {{ exit 1 }}"
            ),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|error| format!("could not inspect PID {process_id}: {error}"))?;
    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        code => Err(format!(
            "PID inspection returned unexpected status {code:?} for {process_id}"
        )),
    }
}

fn stop_pid(process_id: u32) {
    let _ = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            &format!("Stop-Process -Id {process_id} -Force -ErrorAction SilentlyContinue"),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}
