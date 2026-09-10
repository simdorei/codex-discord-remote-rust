use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use cdr_remote_protocol::output::{TerminalWindowEntry, TerminalWindowRect};
use cdr_remote_protocol::request::TerminalWindowShell;
use cdr_windows_native::{
    NativeError, WindowInfo, WindowProcess, activate_window, capture_window_png,
    find_window_by_title_suffix, inspect_window, press_keys, type_text,
};

use super::{
    OwnedTerminalWindow, TerminalWindowBackend, TerminalWindowCapture,
    TerminalWindowInteractionBackend, TerminalWindowObservation,
};
use crate::files::redaction::redact;
use crate::terminal::TerminalError;
use crate::terminal::shell::{display_path, inherited_environment, window_arguments};

const TITLE_ENVIRONMENT: &str = "SIMDOREI_MCP_TERMINAL_WINDOW_TITLE";
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct WindowsTerminalWindowBackend {
    processes: Mutex<HashMap<u64, WindowProcess>>,
}

impl WindowsTerminalWindowBackend {
    pub fn new() -> Self {
        Self {
            processes: Mutex::new(HashMap::new()),
        }
    }

    fn process_id(&self, window: &OwnedTerminalWindow) -> Result<u32, TerminalError> {
        let processes = self.lock()?;
        let process = processes.get(&window.backend_id).ok_or_else(missing)?;
        if process.is_running().map_err(TerminalError::from)? {
            Ok(process.process_id())
        } else {
            Err(missing())
        }
    }

    fn current_info(
        &self,
        window: &OwnedTerminalWindow,
    ) -> Result<Option<WindowInfo>, TerminalError> {
        let process_id = match self.process_id(window) {
            Ok(value) => value,
            Err(TerminalError::Window(message))
                if message == "terminal window is no longer available" =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let expected = window.window_process_id.ok_or_else(missing)?;
        let info =
            inspect_window(window.entry.window_id, Some(expected)).map_err(TerminalError::from)?;
        Ok(info
            .filter(|_| process_id == u32::try_from(window.entry.process_id).unwrap_or_default()))
    }

    fn lock(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, HashMap<u64, WindowProcess>>, TerminalError> {
        self.processes
            .lock()
            .map_err(|_| TerminalError::Window("terminal window process lock was poisoned".into()))
    }
}

impl TerminalWindowBackend for WindowsTerminalWindowBackend {
    fn open(
        &self,
        terminal_window_id: &str,
        shell: TerminalWindowShell,
        cwd: &Path,
        title: &str,
    ) -> Result<OwnedTerminalWindow, TerminalError> {
        let (executable, arguments) = window_arguments(shell, title)?;
        let mut environment = inherited_environment();
        environment.insert(TITLE_ENVIRONMENT.into(), title.into());
        let mut process = WindowProcess::launch(&executable, &arguments, cwd, &environment)
            .map_err(TerminalError::from)?;
        let process_id = process.process_id();
        let info = match find_window_by_title_suffix(title, OPEN_TIMEOUT) {
            Ok(value) => value,
            Err(error) => {
                let cleanup = process.terminate_tree(CLOSE_TIMEOUT);
                return Err(cleanup
                    .err()
                    .map_or_else(|| TerminalError::from(error), TerminalError::from));
            }
        };
        let backend_id = u64::from(process_id);
        self.lock()?.insert(backend_id, process);
        Ok(OwnedTerminalWindow {
            entry: entry(terminal_window_id, shell, cwd, process_id, &info),
            backend_id,
            window_process_id: Some(info.process_id),
        })
    }

    fn inspect(
        &self,
        window: &OwnedTerminalWindow,
    ) -> Result<Option<TerminalWindowEntry>, TerminalError> {
        self.current_info(window).map(|value| {
            value.map(|info| {
                entry(
                    &window.entry.terminal_window_id.0,
                    window.entry.shell,
                    Path::new(&window.entry.cwd),
                    u32::try_from(window.entry.process_id).unwrap_or_default(),
                    &info,
                )
            })
        })
    }

    fn close(&self, window: &OwnedTerminalWindow) -> Result<(), TerminalError> {
        let mut processes = self.lock()?;
        let process = processes.get_mut(&window.backend_id).ok_or_else(missing)?;
        process
            .terminate_tree(CLOSE_TIMEOUT)
            .map_err(TerminalError::from)?;
        processes.remove(&window.backend_id);
        Ok(())
    }
}

impl TerminalWindowInteractionBackend for WindowsTerminalWindowBackend {
    fn capture(
        &self,
        window: &OwnedTerminalWindow,
    ) -> Result<TerminalWindowCapture, TerminalError> {
        let before = self.current_info(window)?.ok_or_else(missing)?;
        let png = capture_window_png(before.window_id, before.rect.width, before.rect.height)
            .map_err(TerminalError::from)?;
        let after = self.current_info(window)?.ok_or_else(missing)?;
        if before.rect != after.rect || before.process_id != after.process_id {
            return Err(TerminalError::Window(
                "terminal window changed during capture".into(),
            ));
        }
        Ok(TerminalWindowCapture {
            rect: protocol_rect(&before),
            png,
        })
    }

    fn activate(&self, window: &OwnedTerminalWindow) -> Result<bool, TerminalError> {
        activate_window(window.entry.window_id, expected_pid(window)?).map_err(TerminalError::from)
    }

    fn type_text(&self, window: &OwnedTerminalWindow, text: &str) -> Result<bool, TerminalError> {
        type_text(window.entry.window_id, expected_pid(window)?, text).map_err(TerminalError::from)
    }

    fn press_keys(
        &self,
        window: &OwnedTerminalWindow,
        keys: &[String],
    ) -> Result<(bool, Vec<String>), TerminalError> {
        press_keys(window.entry.window_id, expected_pid(window)?, keys).map_err(TerminalError::from)
    }

    fn interrupt(&self, window: &OwnedTerminalWindow) -> Result<(), TerminalError> {
        press_keys(
            window.entry.window_id,
            expected_pid(window)?,
            &["CTRL".into(), "C".into()],
        )
        .map(|_| ())
        .map_err(TerminalError::from)
    }

    fn matches_observation(
        &self,
        window: &OwnedTerminalWindow,
        observation: &TerminalWindowObservation,
    ) -> bool {
        self.current_info(window).is_ok_and(|value| {
            value.is_some_and(|info| {
                observation.terminal_window_id == window.entry.terminal_window_id
                    && observation.window_process_id == info.process_id
                    && observation.rect == protocol_rect(&info)
            })
        })
    }
}

fn entry(
    id: &str,
    shell: TerminalWindowShell,
    cwd: &Path,
    process_id: u32,
    info: &WindowInfo,
) -> TerminalWindowEntry {
    TerminalWindowEntry {
        terminal_window_id: cdr_remote_protocol::identifiers::TerminalWindowId(id.into()),
        window_id: info.window_id,
        process_id: u64::from(process_id),
        shell,
        cwd: display_path(cwd),
        title: redact(&info.title).chars().take(500).collect(),
        running: true,
    }
}

fn protocol_rect(info: &WindowInfo) -> TerminalWindowRect {
    TerminalWindowRect {
        left: i64::from(info.rect.left),
        top: i64::from(info.rect.top),
        width: u64::from(info.rect.width),
        height: u64::from(info.rect.height),
    }
}

fn expected_pid(window: &OwnedTerminalWindow) -> Result<u32, TerminalError> {
    window.window_process_id.ok_or_else(missing)
}

fn missing() -> TerminalError {
    TerminalError::Window("terminal window is no longer available".into())
}

impl From<NativeError> for TerminalError {
    fn from(error: NativeError) -> Self {
        Self::Window(error.to_string())
    }
}
