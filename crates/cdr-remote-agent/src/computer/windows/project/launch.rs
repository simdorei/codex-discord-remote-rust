use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cdr_remote_protocol::request::ComputerApp;
use cdr_windows_native::{WindowProcess, enumerate_visible_windows};

use super::{OwnedApplication, STOP_TIMEOUT, WindowsProjectPlatform};
use crate::computer::ComputerError;
use crate::computer::windows::model::{ResolvedWindow, platform, resolve_project, same_window};
use crate::terminal::inherited_environment;

const OPEN_TIMEOUT: Duration = Duration::from_secs(8);
const PROFILE_PREFIX: &str = "simdorei-mcp-chrome-";

impl WindowsProjectPlatform {
    pub(super) fn launch_owned(&self, app: ComputerApp) -> Result<(), ComputerError> {
        if self.lock()?.values().any(|owned| owned.app == app) {
            return Err(platform(
                &"This session already has a launched application window.",
            ));
        }
        let executable = executable(app)?;
        let profile = if app == ComputerApp::Chrome {
            Some(create_profile()?)
        } else {
            None
        };
        let arguments = launch_arguments(app, profile.as_deref());
        let cwd = executable
            .parent()
            .ok_or_else(|| platform(&"application has no parent directory"))?;
        let mut process =
            match WindowProcess::launch(&executable, &arguments, cwd, &inherited_environment()) {
                Ok(value) => value,
                Err(error) => {
                    cleanup_profile(profile.as_deref())?;
                    return Err(platform(&error));
                }
            };
        let resolved = match wait_for_window(process.process_id(), app) {
            Ok(value) => value,
            Err(error) => {
                process
                    .terminate_tree(STOP_TIMEOUT)
                    .map_err(|error| platform(&error))?;
                cleanup_profile(profile.as_deref())?;
                return Err(error);
            }
        };
        self.lock()?.insert(
            resolved.entry.window_id,
            OwnedApplication {
                app,
                process,
                window_process_id: resolved.identity.process_id,
                process_path: resolved.identity.process_path,
                profile,
            },
        );
        Ok(())
    }
}

fn wait_for_window(process_id: u32, app: ComputerApp) -> Result<ResolvedWindow, ComputerError> {
    let deadline = Instant::now() + OPEN_TIMEOUT;
    let mut previous: Option<ResolvedWindow> = None;
    while Instant::now() < deadline {
        for native in enumerate_visible_windows().map_err(|error| platform(&error))? {
            if native.process_id == process_id
                && let Ok(window) = resolve_project(native.window_id)
                && executable_matches(app, &window.identity.process_path)
            {
                if previous.as_ref().is_some_and(|prior| {
                    prior.entry.window_id == window.entry.window_id
                        && same_window(&prior.identity, &window.identity)
                }) {
                    return Ok(window);
                }
                previous = Some(window);
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(platform(
        &"The launched application did not open a safe window in time.",
    ))
}

fn executable(app: ComputerApp) -> Result<PathBuf, ComputerError> {
    let candidates = match app {
        ComputerApp::Notepad => env::var_os("SYSTEMROOT")
            .map(|root| vec![PathBuf::from(root).join("System32/notepad.exe")])
            .unwrap_or_default(),
        ComputerApp::Chrome => ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"]
            .into_iter()
            .filter_map(env::var_os)
            .map(PathBuf::from)
            .map(|base| base.join("Google/Chrome/Application/chrome.exe"))
            .collect(),
    };
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            platform(&"The requested application is not installed in a standard location.")
        })
}

fn launch_arguments(app: ComputerApp, profile: Option<&Path>) -> Vec<String> {
    match app {
        ComputerApp::Notepad => Vec::new(),
        ComputerApp::Chrome => vec![
            format!(
                "--user-data-dir={}",
                profile.unwrap_or(Path::new("")).display()
            ),
            "--guest".into(),
            "--no-first-run".into(),
            "--disable-sync".into(),
            "--disable-background-mode".into(),
            "--new-window".into(),
            "about:blank".into(),
        ],
    }
}

fn create_profile() -> Result<PathBuf, ComputerError> {
    let path = env::temp_dir().join(format!("{PROFILE_PREFIX}{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir(&path).map_err(|error| platform(&error))?;
    Ok(path)
}

pub(super) fn cleanup_profile(profile: Option<&Path>) -> Result<(), ComputerError> {
    let Some(path) = profile else {
        return Ok(());
    };
    let parent = path
        .parent()
        .ok_or_else(|| platform(&"invalid Chrome profile path"))?;
    if parent != env::temp_dir()
        || !path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(PROFILE_PREFIX))
    {
        return Err(platform(
            &"refusing to remove an unrecognized Chrome profile path",
        ));
    }
    if path.exists() {
        fs::remove_dir_all(path).map_err(|error| platform(&error))?;
    }
    Ok(())
}

fn executable_matches(app: ComputerApp, path: &str) -> bool {
    Path::new(path).file_name().is_some_and(|name| {
        name.to_string_lossy().eq_ignore_ascii_case(match app {
            ComputerApp::Chrome => "chrome.exe",
            ComputerApp::Notepad => "notepad.exe",
        })
    })
}
