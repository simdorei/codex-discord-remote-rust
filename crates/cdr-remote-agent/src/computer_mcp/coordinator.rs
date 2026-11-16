//! Serialize participating MCP processes and invalidate their stale observations.
//!
//! Only a random revision is shared on disk. This is coordination, not an app
//! permission grant; native window identity and screenshot TTL checks still run.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
#[cfg(any(windows, test))]
use std::time::{Duration, Instant};

use cdr_core::Validate;
use cdr_remote_protocol::output::ComputerOutput;
use cdr_remote_protocol::request::ComputerRequest;
use uuid::Uuid;

use crate::computer::ComputerError;

const GATE_NAME: &str = "Local\\CodexDiscordRemoteComputerMcpAction";
const EPOCH_FILE: &str = "codex-discord-remote-computer-v1.epoch";
#[cfg(any(windows, test))]
const GATE_WAIT: Duration = Duration::from_secs(10);
#[cfg(any(windows, test))]
const GATE_POLL: Duration = Duration::from_millis(25);

pub(super) struct Coordinator {
    epoch_path: Option<PathBuf>,
    gate_name: String,
    observation: Option<(String, Uuid)>,
}

impl Coordinator {
    pub(super) fn new() -> Self {
        // A per-task TEMP/TMP override must not split processes into separate
        // revision files. Resolve one user-owned path once, without file I/O.
        let epoch_path = if cfg!(windows) {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .filter(|root| root.is_absolute())
                .map(|root| {
                    root.join("CodexDiscordRemote/computer-use")
                        .join(EPOCH_FILE)
                })
        } else {
            None
        };
        Self {
            epoch_path,
            gate_name: GATE_NAME.into(),
            observation: None,
        }
    }

    pub(super) fn execute(
        &mut self,
        request: &ComputerRequest,
        action: impl FnOnce(&ComputerRequest) -> Result<ComputerOutput, ComputerError>,
    ) -> Result<ComputerOutput, ComputerError> {
        self.epoch_path()?;
        if matches!(request, ComputerRequest::ComputerListWindows) {
            return action(request);
        }
        // A failed capture, blocked gate, or attempted input consumes this
        // process's observation even when the native operation never begins.
        let previous = self.observation.take();
        let _gate = acquire_gate(&self.gate_name)?;
        let parent = self.epoch_path()?.parent().ok_or(ComputerError::State)?;
        fs::create_dir_all(parent).map_err(|error| state_io(&error))?;
        if let ComputerRequest::ComputerScreenshot { window_id } = request {
            let epoch = self.read_epoch()?;
            let output = action(request)?;
            output.validate().map_err(|_| invalid_snapshot())?;
            let ComputerOutput::ComputerScreenshot {
                observation_id,
                window,
                ..
            } = &output
            else {
                return Err(invalid_snapshot());
            };
            if window.window_id != *window_id {
                return Err(ComputerError::DifferentWindow);
            }
            self.observation = Some((observation_id.clone(), epoch));
            return Ok(output);
        }
        if let Some(id) = observed_id(request) {
            let (previous_id, previous_epoch) = previous.ok_or(ComputerError::FreshScreenshot)?;
            if previous_id != id || previous_epoch != self.read_epoch()? {
                return Err(ComputerError::FreshScreenshot);
            }
        }
        // Publish before any operation that can affect the desktop, even if
        // that operation later fails after partially applying an input.
        // Stop does not require the old revision to be readable or valid.
        self.publish_epoch()?;
        action(request)
    }

    fn epoch_path(&self) -> Result<&Path, ComputerError> {
        self.epoch_path
            .as_deref()
            .filter(|path| path.is_absolute())
            .ok_or_else(|| {
                ComputerError::Platform(
                    "An absolute Windows LOCALAPPDATA directory is required for desktop coordination."
                        .into(),
                )
            })
    }

    fn read_epoch(&self) -> Result<Uuid, ComputerError> {
        let file = match File::open(self.epoch_path()?) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return self.publish_epoch();
            }
            Err(error) => return Err(state_io(&error)),
        };
        // Never read unbounded data from a coordination file changed by another
        // process. Only our exact, lowercase, simple UUID-v4 format is accepted.
        let mut bytes = Vec::with_capacity(33);
        file.take(33)
            .read_to_end(&mut bytes)
            .map_err(|error| state_io(&error))?;
        let parsed = std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| Uuid::parse_str(text).ok())
            .filter(|epoch| {
                epoch.get_version_num() == 4
                    && epoch.get_variant() == uuid::Variant::RFC4122
                    && epoch.simple().to_string().as_bytes() == bytes.as_slice()
            });
        parsed.ok_or_else(|| {
            ComputerError::Platform(
                "The shared desktop coordination state is invalid; no input was sent.".into(),
            )
        })
    }

    fn publish_epoch(&self) -> Result<Uuid, ComputerError> {
        let epoch = Uuid::new_v4();
        let path = self.epoch_path()?;
        let temporary = path.with_extension(format!("{}.tmp", Uuid::new_v4().simple()));
        let mut created = false;
        let result = (|| -> io::Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            created = true;
            file.write_all(epoch.simple().to_string().as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, path)
        })();
        if result.is_err() && created {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(|error| state_io(&error))?;
        Ok(epoch)
    }
}

fn observed_id(request: &ComputerRequest) -> Option<&str> {
    match request {
        ComputerRequest::ComputerClick { observation_id, .. }
        | ComputerRequest::ComputerDrag { observation_id, .. }
        | ComputerRequest::ComputerScroll { observation_id, .. }
        | ComputerRequest::ComputerTypeText { observation_id, .. }
        | ComputerRequest::ComputerPressKeys { observation_id, .. }
        | ComputerRequest::ComputerClose { observation_id, .. }
        | ComputerRequest::ComputerSetClipboard { observation_id, .. } => Some(observation_id),
        ComputerRequest::ComputerListWindows
        | ComputerRequest::ComputerActivate { .. }
        | ComputerRequest::ComputerLaunch { .. }
        | ComputerRequest::ComputerScreenshot { .. }
        | ComputerRequest::ComputerStop => None,
    }
}

fn state_io(error: &io::Error) -> ComputerError {
    ComputerError::Platform(format!("Desktop coordination is unavailable: {error}"))
}

fn invalid_snapshot() -> ComputerError {
    ComputerError::Platform("The computer backend returned an invalid screenshot.".into())
}

#[cfg(windows)]
fn acquire_gate(name: &str) -> Result<cdr_windows_native::SingleInstance, ComputerError> {
    let deadline = Instant::now() + GATE_WAIT;
    loop {
        match cdr_windows_native::SingleInstance::acquire(name) {
            Ok(gate) => return Ok(gate),
            Err(cdr_windows_native::NativeError::AlreadyRunning) => {
                if Instant::now() >= deadline {
                    return Err(ComputerError::Platform(
                        "Another computer operation is busy; take a fresh screenshot and retry."
                            .into(),
                    ));
                }
                std::thread::sleep(GATE_POLL);
            }
            Err(error) => return Err(ComputerError::Platform(error.to_string())),
        }
    }
}

#[cfg(all(not(windows), not(test)))]
fn acquire_gate(_: &str) -> Result<(), ComputerError> {
    Err(ComputerError::Platform(
        "Computer control is currently available only on Windows.".into(),
    ))
}

#[cfg(all(not(windows), test))]
fn acquire_gate(_: &str) -> Result<std::sync::MutexGuard<'static, ()>, ComputerError> {
    use std::sync::{Mutex, TryLockError};
    static GATE: Mutex<()> = Mutex::new(());
    let deadline = Instant::now() + GATE_WAIT;
    loop {
        match GATE.try_lock() {
            Ok(gate) => return Ok(gate),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(GATE_POLL);
            }
            Err(_) => return Err(ComputerError::State),
        }
    }
}

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;
