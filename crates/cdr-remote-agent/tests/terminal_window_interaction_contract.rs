use std::path::Path;
use std::sync::{Arc, Mutex};

use cdr_remote_agent::terminal::{
    OwnedTerminalWindow, TerminalError, TerminalWindowBackend, TerminalWindowCapture,
    TerminalWindowInteractionBackend, TerminalWindowManager, TerminalWindowObservation,
};
use cdr_remote_protocol::identifiers::{TerminalWindowId, TerminalWindowObservationId};
use cdr_remote_protocol::output::{
    TerminalOutput, TerminalWindowAction, TerminalWindowEntry, TerminalWindowRect,
};
use cdr_remote_protocol::request::{TerminalRequest, TerminalWindowShell};

struct Lifecycle;

impl TerminalWindowBackend for Lifecycle {
    fn open(
        &self,
        id: &str,
        shell: TerminalWindowShell,
        cwd: &Path,
        title: &str,
    ) -> Result<OwnedTerminalWindow, TerminalError> {
        Ok(OwnedTerminalWindow {
            entry: TerminalWindowEntry {
                terminal_window_id: TerminalWindowId(id.into()),
                window_id: 1_201,
                process_id: 201,
                shell,
                cwd: cwd.display().to_string(),
                title: title.into(),
                running: true,
            },
            backend_id: 201,
            window_process_id: Some(2_201),
        })
    }
    fn inspect(
        &self,
        window: &OwnedTerminalWindow,
    ) -> Result<Option<TerminalWindowEntry>, TerminalError> {
        Ok(Some(window.entry.clone()))
    }
    fn close(&self, _window: &OwnedTerminalWindow) -> Result<(), TerminalError> {
        Ok(())
    }
}

#[derive(Default)]
struct InteractionState {
    typed: Vec<String>,
    interrupts: u32,
    matches: bool,
    fail_type: bool,
}

struct Interaction(Arc<Mutex<InteractionState>>);

impl TerminalWindowInteractionBackend for Interaction {
    fn capture(
        &self,
        _window: &OwnedTerminalWindow,
    ) -> Result<TerminalWindowCapture, TerminalError> {
        Ok(TerminalWindowCapture {
            rect: TerminalWindowRect {
                left: 10,
                top: 20,
                width: 640,
                height: 480,
            },
            png: b"\x89PNG\r\n\x1a\nsynthetic".to_vec(),
        })
    }
    fn activate(&self, _window: &OwnedTerminalWindow) -> Result<bool, TerminalError> {
        Ok(true)
    }
    fn type_text(&self, _window: &OwnedTerminalWindow, text: &str) -> Result<bool, TerminalError> {
        let mut state = self.0.lock().expect("state");
        if state.fail_type {
            return Err(TerminalError::Window("synthetic input failure".into()));
        }
        state.typed.push(text.into());
        Ok(false)
    }
    fn press_keys(
        &self,
        _window: &OwnedTerminalWindow,
        keys: &[String],
    ) -> Result<(bool, Vec<String>), TerminalError> {
        Ok((
            false,
            keys.iter().map(|key| key.to_ascii_uppercase()).collect(),
        ))
    }
    fn interrupt(&self, _window: &OwnedTerminalWindow) -> Result<(), TerminalError> {
        self.0.lock().expect("state").interrupts += 1;
        Ok(())
    }
    fn matches_observation(
        &self,
        window: &OwnedTerminalWindow,
        observation: &TerminalWindowObservation,
    ) -> bool {
        self.0.lock().expect("state").matches
            && window.window_process_id == Some(observation.window_process_id)
    }
}

fn manager(
    root: &Path,
) -> (
    TerminalWindowManager,
    Arc<Mutex<InteractionState>>,
    TerminalWindowId,
) {
    let state = Arc::new(Mutex::new(InteractionState {
        matches: true,
        ..InteractionState::default()
    }));
    let manager = TerminalWindowManager::with_backends(
        root,
        Arc::new(Lifecycle),
        Arc::new(Interaction(Arc::clone(&state))),
    )
    .expect("manager");
    let opened = manager
        .execute(&TerminalRequest::TerminalWindowOpen {
            shell: TerminalWindowShell::Cmd,
            cwd: None,
        })
        .expect("open");
    let TerminalOutput::TerminalWindowOpen { window } = opened else {
        panic!("expected open")
    };
    (manager, state, window.terminal_window_id)
}

fn capture(manager: &TerminalWindowManager, id: &TerminalWindowId) -> TerminalWindowObservationId {
    match manager
        .execute(&TerminalRequest::TerminalWindowCapture {
            terminal_window_id: id.clone(),
        })
        .expect("capture")
    {
        TerminalOutput::TerminalWindowCapture { observation_id, .. } => observation_id,
        _ => panic!("expected capture"),
    }
}

#[test]
fn tw3_capture_then_type_is_receipted_without_echoing_text_and_cannot_replay() {
    let root = tempfile::tempdir().expect("root");
    let (manager, state, id) = manager(root.path());
    let observation_id = capture(&manager, &id);
    let secret_text = "not-stored-in-receipt";
    let request = TerminalRequest::TerminalWindowType {
        terminal_window_id: id,
        observation_id: observation_id.clone(),
        text: secret_text.into(),
    };
    let output = manager.execute(&request).expect("type");
    let TerminalOutput::TerminalWindowAction { receipt, .. } = output else {
        panic!("expected action")
    };
    assert_eq!(state.lock().expect("state").typed, vec![secret_text]);
    assert_eq!(receipt.action, TerminalWindowAction::Type);
    assert_eq!(receipt.unicode_chars, 21);
    assert_eq!(receipt.observation_id, Some(observation_id));
    assert!(
        !serde_json::to_string(&receipt)
            .expect("json")
            .contains(secret_text)
    );
    assert!(matches!(
        manager.execute(&request),
        Err(TerminalError::Window(message)) if message.contains("fresh capture")
    ));
}

#[test]
fn tw4_keys_interrupt_activate_and_failed_input_consume_observations() {
    let root = tempfile::tempdir().expect("root");
    let (manager, state, id) = manager(root.path());
    let keys = manager
        .execute(&TerminalRequest::TerminalWindowKeys {
            terminal_window_id: id.clone(),
            observation_id: capture(&manager, &id),
            keys: vec!["ctrl".into(), "l".into()],
        })
        .expect("keys");
    let TerminalOutput::TerminalWindowAction { receipt, .. } = keys else {
        panic!("expected keys")
    };
    assert_eq!(receipt.keys, ["CTRL", "L"]);
    let interrupted = manager
        .execute(&TerminalRequest::TerminalWindowInterrupt {
            terminal_window_id: id.clone(),
            observation_id: capture(&manager, &id),
        })
        .expect("interrupt");
    let TerminalOutput::TerminalWindowAction { receipt, .. } = interrupted else {
        panic!("expected interrupt")
    };
    assert_eq!(receipt.keys, ["CTRL", "C"]);
    assert_eq!(state.lock().expect("state").interrupts, 1);

    let observation_id = capture(&manager, &id);
    state.lock().expect("state").fail_type = true;
    let failed = TerminalRequest::TerminalWindowType {
        terminal_window_id: id.clone(),
        observation_id,
        text: "failure".into(),
    };
    assert!(manager.execute(&failed).is_err());
    state.lock().expect("state").fail_type = false;
    assert!(matches!(
        manager.execute(&failed),
        Err(TerminalError::Window(message)) if message.contains("fresh capture")
    ));
    assert!(matches!(
        manager
            .execute(&TerminalRequest::TerminalWindowActivate { terminal_window_id: id })
            .expect("activate"),
        TerminalOutput::TerminalWindowAction { receipt, .. }
            if receipt.action == TerminalWindowAction::Activate && receipt.observation_id.is_none()
    ));
}
