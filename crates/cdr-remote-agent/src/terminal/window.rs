mod interactions;
mod types;
#[cfg(not(windows))]
mod unsupported;
#[cfg(windows)]
mod windows;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use cdr_remote_protocol::identifiers::TerminalWindowId;
use cdr_remote_protocol::output::TerminalOutput;
use cdr_remote_protocol::request::{TerminalRequest, TerminalWindowShell};
use uuid::Uuid;

use super::TerminalError;
use super::shell::resolve_cwd;
pub use types::{
    OwnedTerminalWindow, TerminalWindowBackend, TerminalWindowCapture,
    TerminalWindowInteractionBackend, TerminalWindowObservation,
};

pub struct TerminalWindowManager {
    root: std::path::PathBuf,
    lifecycle: Arc<dyn TerminalWindowBackend>,
    interaction: Arc<dyn TerminalWindowInteractionBackend>,
    state: Mutex<WindowState>,
}

pub(super) struct WindowState {
    pub windows: HashMap<String, OwnedTerminalWindow>,
    pub observations: HashMap<String, TerminalWindowObservation>,
    pub closed: bool,
}

impl TerminalWindowManager {
    pub fn new(root: &Path) -> Result<Self, TerminalError> {
        #[cfg(windows)]
        {
            let backend = Arc::new(windows::WindowsTerminalWindowBackend::new());
            Self::with_backends(root, backend.clone(), backend)
        }
        #[cfg(not(windows))]
        {
            let backend = Arc::new(unsupported::UnsupportedWindowBackend);
            Self::with_backends(root, backend.clone(), backend)
        }
    }

    pub fn with_backends(
        root: &Path,
        lifecycle: Arc<dyn TerminalWindowBackend>,
        interaction: Arc<dyn TerminalWindowInteractionBackend>,
    ) -> Result<Self, TerminalError> {
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err(TerminalError::InvalidRoot);
        }
        Ok(Self {
            root,
            lifecycle,
            interaction,
            state: Mutex::new(WindowState {
                windows: HashMap::new(),
                observations: HashMap::new(),
                closed: false,
            }),
        })
    }

    pub fn execute(&self, request: &TerminalRequest) -> Result<TerminalOutput, TerminalError> {
        match request {
            TerminalRequest::TerminalWindowOpen { shell, cwd } => self.open(*shell, cwd.as_deref()),
            TerminalRequest::TerminalWindowList => self.list(),
            TerminalRequest::TerminalWindowClose { terminal_window_id } => {
                self.close(terminal_window_id)
            }
            TerminalRequest::TerminalWindowCapture { terminal_window_id } => {
                self.capture(terminal_window_id)
            }
            TerminalRequest::TerminalWindowActivate { terminal_window_id } => {
                self.activate(terminal_window_id)
            }
            TerminalRequest::TerminalWindowType {
                terminal_window_id,
                observation_id,
                text,
            } => self.type_text(terminal_window_id, observation_id, text),
            TerminalRequest::TerminalWindowKeys {
                terminal_window_id,
                observation_id,
                keys,
            } => self.press_keys(terminal_window_id, observation_id, keys),
            TerminalRequest::TerminalWindowInterrupt {
                terminal_window_id,
                observation_id,
            } => self.interrupt(terminal_window_id, observation_id),
            TerminalRequest::TerminalExec { .. } => Err(TerminalError::UnsupportedRequest),
        }
    }

    pub fn close_all(&self) -> Result<(), TerminalError> {
        let mut state = self.lock()?;
        state.closed = true;
        let ids = state.windows.keys().cloned().collect::<Vec<_>>();
        let mut first_error = None;
        for id in ids {
            let window = state.windows.get(&id).cloned().expect("known window");
            state.observations.remove(&id);
            match self.lifecycle.close(&window) {
                Ok(()) => {
                    state.windows.remove(&id);
                }
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn open(
        &self,
        shell: TerminalWindowShell,
        requested_cwd: Option<&str>,
    ) -> Result<TerminalOutput, TerminalError> {
        let mut state = self.lock_open()?;
        let cwd = resolve_cwd(&self.root, requested_cwd)?;
        let id = generated_id("termwin_");
        let title = format!("Codex Pro Terminal {id} {}", random_hex());
        let owned = self.lifecycle.open(&id, shell, &cwd, &title)?;
        let window = owned.entry.clone();
        state.windows.insert(id, owned);
        Ok(TerminalOutput::TerminalWindowOpen { window })
    }

    fn list(&self) -> Result<TerminalOutput, TerminalError> {
        let mut state = self.lock_open()?;
        let ids = state.windows.keys().cloned().collect::<Vec<_>>();
        let mut windows = Vec::new();
        for id in ids {
            let owned = state.windows.get(&id).cloned().expect("known window");
            if let Some(entry) = self.lifecycle.inspect(&owned)? {
                windows.push(entry);
                continue;
            }
            state.observations.remove(&id);
            self.lifecycle.close(&owned)?;
            state.windows.remove(&id);
        }
        windows.sort_by(|left, right| left.terminal_window_id.0.cmp(&right.terminal_window_id.0));
        Ok(TerminalOutput::TerminalWindowList { windows })
    }

    fn close(&self, id: &TerminalWindowId) -> Result<TerminalOutput, TerminalError> {
        let mut state = self.lock_open()?;
        let owned = state.windows.get(&id.0).cloned().ok_or_else(|| {
            TerminalError::Window("terminal window does not belong to this session".into())
        })?;
        state.observations.remove(&id.0);
        self.lifecycle.close(&owned)?;
        state.windows.remove(&id.0);
        Ok(TerminalOutput::TerminalWindowClose {
            terminal_window_id: id.clone(),
            closed: true,
        })
    }

    pub(super) fn lock(&self) -> Result<MutexGuard<'_, WindowState>, TerminalError> {
        self.state
            .lock()
            .map_err(|_| TerminalError::Window("terminal window state lock was poisoned".into()))
    }

    pub(super) fn lock_open(&self) -> Result<MutexGuard<'_, WindowState>, TerminalError> {
        let state = self.lock()?;
        if state.closed {
            Err(TerminalError::Window(
                "terminal window session is closed".into(),
            ))
        } else {
            Ok(state)
        }
    }
}

pub(super) fn generated_id(prefix: &str) -> String {
    format!("{prefix}{}", random_hex())
}

fn random_hex() -> String {
    let value = Uuid::new_v4().simple().to_string();
    value[..16].to_owned()
}
