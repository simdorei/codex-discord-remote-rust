#![cfg(windows)]

use std::thread;
use std::time::Duration;

use cdr_remote_agent::terminal::{TerminalError, TerminalWindowManager};
use cdr_remote_protocol::identifiers::{TerminalWindowId, TerminalWindowObservationId};
use cdr_remote_protocol::output::TerminalOutput;
use cdr_remote_protocol::request::{TerminalRequest, TerminalWindowShell};

#[test]
#[ignore = "opens and briefly activates real Windows terminal windows"]
fn current_windows_terminal_backend_opens_captures_inputs_and_closes_owned_trees() {
    let root = tempfile::tempdir().expect("root");
    let manager = TerminalWindowManager::new(root.path()).expect("manager");
    let result = live_round_trip(&manager);
    let cleanup = manager.close_all();
    assert!(result.is_ok(), "live window round trip failed: {result:?}");
    assert!(cleanup.is_ok(), "live window cleanup failed: {cleanup:?}");
}

fn live_round_trip(manager: &TerminalWindowManager) -> Result<(), TerminalError> {
    let cmd = open(manager, TerminalWindowShell::Cmd)?;
    let powershell = open(manager, TerminalWindowShell::Powershell)?;
    let listed = manager.execute(&TerminalRequest::TerminalWindowList)?;
    assert!(matches!(
        listed,
        TerminalOutput::TerminalWindowList { ref windows }
            if windows.len() == 2
                && windows.iter().any(|window| window.terminal_window_id == cmd)
                && windows.iter().any(|window| window.terminal_window_id == powershell)
    ));
    let observation = capture(manager, &cmd)?;
    let typed = manager.execute(&TerminalRequest::TerminalWindowType {
        terminal_window_id: cmd.clone(),
        observation_id: observation,
        text: "echo rust-window-smoke".into(),
    })?;
    assert!(matches!(typed, TerminalOutput::TerminalWindowAction { .. }));
    let observation = capture(manager, &cmd)?;
    let entered = manager.execute(&TerminalRequest::TerminalWindowKeys {
        terminal_window_id: cmd.clone(),
        observation_id: observation,
        keys: vec!["ENTER".into()],
    })?;
    assert!(matches!(
        entered,
        TerminalOutput::TerminalWindowAction { .. }
    ));
    thread::sleep(Duration::from_millis(300));
    let _ = capture(manager, &cmd)?;
    Ok(())
}

fn open(
    manager: &TerminalWindowManager,
    shell: TerminalWindowShell,
) -> Result<TerminalWindowId, TerminalError> {
    match manager.execute(&TerminalRequest::TerminalWindowOpen { shell, cwd: None })? {
        TerminalOutput::TerminalWindowOpen { window } => Ok(window.terminal_window_id),
        _ => unreachable!(),
    }
}

fn capture(
    manager: &TerminalWindowManager,
    id: &TerminalWindowId,
) -> Result<TerminalWindowObservationId, TerminalError> {
    match manager.execute(&TerminalRequest::TerminalWindowCapture {
        terminal_window_id: id.clone(),
    })? {
        TerminalOutput::TerminalWindowCapture {
            observation_id,
            data_base64,
            ..
        } => {
            assert!(data_base64.starts_with("iVBOR"));
            Ok(observation_id)
        }
        _ => unreachable!(),
    }
}
