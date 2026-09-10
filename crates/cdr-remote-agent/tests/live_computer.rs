#![cfg(windows)]

use cdr_remote_agent::computer::{ComputerAccessMode, ComputerController, new_windows_controller};
use cdr_remote_protocol::output::{ComputerActionName, ComputerOutput, ComputerWindowEntry};
use cdr_remote_protocol::request::{ComputerApp, ComputerRequest};
use std::time::Duration;

#[test]
#[ignore = "opens and briefly controls a session-owned Notepad window"]
fn current_windows_project_computer_launches_captures_types_and_stops() {
    let controller = new_windows_controller(ComputerAccessMode::Project);
    let launch = controller
        .execute(&ComputerRequest::ComputerLaunch {
            app: ComputerApp::Notepad,
        })
        .expect("launch Notepad");
    assert!(matches!(
        launch,
        ComputerOutput::ComputerAction {
            action: ComputerActionName::Launch,
            ..
        }
    ));

    let window = only_window(&controller);
    controller
        .execute(&ComputerRequest::ComputerActivate {
            window_id: window.window_id,
        })
        .expect("activate Notepad");
    let observation = screenshot(&controller, window.window_id);
    let typed = controller
        .execute(&ComputerRequest::ComputerTypeText {
            window_id: window.window_id,
            observation_id: observation,
            text: "rust-computer-live-smoke".into(),
        })
        .expect("type into owned Notepad");
    let encoded = serde_json::to_string(&typed).expect("serialize receipt");
    assert!(!encoded.contains("rust-computer-live-smoke"));

    std::thread::sleep(Duration::from_millis(300));
    let _ = screenshot(&controller, window.window_id);
    controller
        .execute(&ComputerRequest::ComputerStop)
        .expect("stop and terminate the owned process tree");
}

fn only_window(controller: &ComputerController) -> ComputerWindowEntry {
    match controller
        .execute(&ComputerRequest::ComputerListWindows)
        .expect("list owned windows")
    {
        ComputerOutput::ComputerWindows { windows } => {
            assert_eq!(windows.len(), 1);
            windows.into_iter().next().expect("owned Notepad")
        }
        _ => panic!("expected computer windows"),
    }
}

fn screenshot(controller: &ComputerController, window_id: u64) -> String {
    match controller
        .execute(&ComputerRequest::ComputerScreenshot { window_id })
        .expect("capture owned Notepad")
    {
        ComputerOutput::ComputerScreenshot {
            observation_id,
            data_base64,
            ..
        } => {
            assert!(data_base64.starts_with("iVBOR"));
            observation_id
        }
        _ => panic!("expected computer screenshot"),
    }
}
