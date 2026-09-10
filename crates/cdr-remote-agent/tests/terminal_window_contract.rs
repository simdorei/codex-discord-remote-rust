use std::path::Path;
use std::sync::{Arc, Mutex};

use cdr_remote_agent::terminal::{
    OwnedTerminalWindow, TerminalError, TerminalWindowBackend, TerminalWindowCapture,
    TerminalWindowInteractionBackend, TerminalWindowManager, TerminalWindowObservation,
};
use cdr_remote_protocol::output::{TerminalOutput, TerminalWindowEntry, TerminalWindowRect};
use cdr_remote_protocol::request::{TerminalRequest, TerminalWindowShell};

#[derive(Default)]
struct LifecycleState {
    next_pid: u32,
    closed: Vec<String>,
    stale: Vec<String>,
    fail_open: bool,
    fail_close_once: bool,
}

struct FakeLifecycle(Arc<Mutex<LifecycleState>>);

impl TerminalWindowBackend for FakeLifecycle {
    fn open(
        &self,
        terminal_window_id: &str,
        shell: TerminalWindowShell,
        cwd: &Path,
        title: &str,
    ) -> Result<OwnedTerminalWindow, TerminalError> {
        let mut state = self.0.lock().expect("state");
        if state.fail_open {
            return Err(TerminalError::Window("synthetic launch failure".into()));
        }
        state.next_pid += 1;
        let pid = 100 + state.next_pid;
        Ok(OwnedTerminalWindow {
            entry: TerminalWindowEntry {
                terminal_window_id: cdr_remote_protocol::identifiers::TerminalWindowId(
                    terminal_window_id.into(),
                ),
                window_id: u64::from(pid + 1_000),
                process_id: u64::from(pid),
                shell,
                cwd: cwd.display().to_string(),
                title: title.into(),
                running: true,
            },
            backend_id: u64::from(pid),
            window_process_id: Some(pid + 2_000),
        })
    }

    fn inspect(
        &self,
        window: &OwnedTerminalWindow,
    ) -> Result<Option<TerminalWindowEntry>, TerminalError> {
        let state = self.0.lock().expect("state");
        Ok((!state.stale.contains(&window.entry.terminal_window_id.0))
            .then(|| window.entry.clone()))
    }

    fn close(&self, window: &OwnedTerminalWindow) -> Result<(), TerminalError> {
        let mut state = self.0.lock().expect("state");
        if state.fail_close_once {
            state.fail_close_once = false;
            return Err(TerminalError::Window("synthetic close failure".into()));
        }
        state.closed.push(window.entry.terminal_window_id.0.clone());
        Ok(())
    }
}

struct NoInteraction;

impl TerminalWindowInteractionBackend for NoInteraction {
    fn capture(
        &self,
        _window: &OwnedTerminalWindow,
    ) -> Result<TerminalWindowCapture, TerminalError> {
        Err(TerminalError::Window("not used".into()))
    }
    fn activate(&self, _window: &OwnedTerminalWindow) -> Result<bool, TerminalError> {
        Err(TerminalError::Window("not used".into()))
    }
    fn type_text(&self, _window: &OwnedTerminalWindow, _text: &str) -> Result<bool, TerminalError> {
        Err(TerminalError::Window("not used".into()))
    }
    fn press_keys(
        &self,
        _window: &OwnedTerminalWindow,
        _keys: &[String],
    ) -> Result<(bool, Vec<String>), TerminalError> {
        Err(TerminalError::Window("not used".into()))
    }
    fn interrupt(&self, _window: &OwnedTerminalWindow) -> Result<(), TerminalError> {
        Err(TerminalError::Window("not used".into()))
    }
    fn matches_observation(
        &self,
        _window: &OwnedTerminalWindow,
        _observation: &TerminalWindowObservation,
    ) -> bool {
        false
    }
}

fn manager(root: &Path, state: Arc<Mutex<LifecycleState>>) -> TerminalWindowManager {
    TerminalWindowManager::with_backends(
        root,
        Arc::new(FakeLifecycle(state)),
        Arc::new(NoInteraction),
    )
    .expect("manager")
}

fn open(shell: TerminalWindowShell, cwd: Option<String>) -> TerminalRequest {
    TerminalRequest::TerminalWindowOpen { shell, cwd }
}

#[test]
fn tw1_manager_opens_lists_closes_and_prunes_only_owned_windows() {
    let root = tempfile::tempdir().expect("root");
    let state = Arc::new(Mutex::new(LifecycleState::default()));
    let manager = manager(root.path(), Arc::clone(&state));
    let first = manager
        .execute(&open(TerminalWindowShell::Powershell, None))
        .expect("powershell");
    let second = manager
        .execute(&open(
            TerminalWindowShell::Cmd,
            Some(root.path().display().to_string()),
        ))
        .expect("cmd");
    let first_id = opened_id(&first);
    let second_id = opened_id(&second);
    assert_ne!(first_id, second_id);
    assert_eq!(listed(&manager).len(), 2);

    state.lock().expect("state").stale.push(first_id.clone());
    let remaining = listed(&manager);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].terminal_window_id.0, second_id);
    assert_eq!(state.lock().expect("state").closed, vec![first_id]);
    manager.close_all().expect("close all");
    assert_eq!(state.lock().expect("state").closed.len(), 2);
}

#[test]
fn tw2_failed_launch_is_not_registered_and_failed_cleanup_is_retryable() {
    let root = tempfile::tempdir().expect("root");
    let state = Arc::new(Mutex::new(LifecycleState::default()));
    let manager = manager(root.path(), Arc::clone(&state));
    state.lock().expect("state").fail_open = true;
    assert!(matches!(
        manager.execute(&open(TerminalWindowShell::Powershell, None)),
        Err(TerminalError::Window(message)) if message == "synthetic launch failure"
    ));
    state.lock().expect("state").fail_open = false;
    assert!(listed(&manager).is_empty());
    manager
        .execute(&open(TerminalWindowShell::Cmd, None))
        .expect("open");
    state.lock().expect("state").fail_close_once = true;
    assert!(manager.close_all().is_err());
    manager.close_all().expect("retry cleanup");
    assert_eq!(state.lock().expect("state").closed.len(), 1);
}

fn opened_id(output: &TerminalOutput) -> String {
    match output {
        TerminalOutput::TerminalWindowOpen { window } => window.terminal_window_id.0.clone(),
        _ => panic!("expected open output"),
    }
}

fn listed(manager: &TerminalWindowManager) -> Vec<TerminalWindowEntry> {
    match manager
        .execute(&TerminalRequest::TerminalWindowList)
        .expect("list")
    {
        TerminalOutput::TerminalWindowList { windows } => windows,
        _ => panic!("expected list output"),
    }
}

#[allow(dead_code)]
fn capture_fixture() -> TerminalWindowCapture {
    TerminalWindowCapture {
        rect: TerminalWindowRect {
            left: 10,
            top: 20,
            width: 640,
            height: 480,
        },
        png: b"\x89PNG\r\n\x1a\nsynthetic".to_vec(),
    }
}
