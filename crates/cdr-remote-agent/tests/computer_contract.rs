use std::sync::{Arc, Mutex};
use std::time::Duration;

use cdr_remote_agent::computer::{
    ComputerCapture, ComputerController, ComputerError, ComputerIdentity, ComputerPlatform,
};
use cdr_remote_protocol::output::{ComputerActionName, ComputerOutput, ComputerWindowEntry};
use cdr_remote_protocol::request::{ComputerApp, ComputerMouseButton, ComputerRequest};

#[derive(Default)]
struct Calls {
    typed: Vec<String>,
    clicks: Vec<(u32, u32)>,
    stops: u32,
}

struct FakePlatform(Arc<Mutex<Calls>>);

impl ComputerPlatform for FakePlatform {
    fn list_windows(&self) -> Result<Vec<ComputerWindowEntry>, ComputerError> {
        Ok(vec![window()])
    }
    fn screenshot(&self, window_id: u64) -> Result<ComputerCapture, ComputerError> {
        if window_id != 42 {
            return Err(ComputerError::Platform("missing window".into()));
        }
        Ok(ComputerCapture {
            window: window(),
            identity: identity(),
            png: b"\x89PNG\r\n\x1a\nfixture".to_vec(),
        })
    }
    fn activate(&self, _window_id: u64) -> Result<ComputerWindowEntry, ComputerError> {
        Ok(window())
    }
    fn launch(&self, _app: ComputerApp) -> Result<(), ComputerError> {
        Ok(())
    }
    fn click(
        &self,
        _identity: &ComputerIdentity,
        x: u32,
        y: u32,
        _button: ComputerMouseButton,
        _count: u8,
    ) -> Result<(), ComputerError> {
        self.0.lock().expect("calls").clicks.push((x, y));
        Ok(())
    }
    fn drag(
        &self,
        _identity: &ComputerIdentity,
        _start: (u32, u32),
        _end: (u32, u32),
    ) -> Result<(), ComputerError> {
        Ok(())
    }
    fn scroll(
        &self,
        _identity: &ComputerIdentity,
        _point: (u32, u32),
        _delta: (i32, i32),
    ) -> Result<(), ComputerError> {
        Ok(())
    }
    fn type_text(&self, _identity: &ComputerIdentity, text: &str) -> Result<(), ComputerError> {
        self.0.lock().expect("calls").typed.push(text.into());
        Ok(())
    }
    fn press_keys(
        &self,
        _identity: &ComputerIdentity,
        _keys: &[String],
    ) -> Result<(), ComputerError> {
        Ok(())
    }
    fn close(&self, _identity: &ComputerIdentity) -> Result<(), ComputerError> {
        Ok(())
    }
    fn set_clipboard(
        &self,
        _identity: &ComputerIdentity,
        _text: &str,
    ) -> Result<(), ComputerError> {
        Ok(())
    }
    fn stop(&self) -> Result<(), ComputerError> {
        self.0.lock().expect("calls").stops += 1;
        Ok(())
    }
}

fn controller(ttl: Duration) -> (ComputerController, Arc<Mutex<Calls>>) {
    let calls = Arc::new(Mutex::new(Calls::default()));
    (
        ComputerController::with_platform_and_ttl(Arc::new(FakePlatform(Arc::clone(&calls))), ttl),
        calls,
    )
}

fn window() -> ComputerWindowEntry {
    ComputerWindowEntry {
        window_id: 42,
        title: "Fixture".into(),
        process_name: "fixture.exe".into(),
        left: 10,
        top: 20,
        width: 640,
        height: 480,
        active: true,
    }
}

fn identity() -> ComputerIdentity {
    ComputerIdentity {
        window_id: 42,
        process_id: 123,
        process_path: r"c:\fixture.exe".into(),
        title_digest: "a".repeat(64),
        left: 10,
        top: 20,
        width: 640,
        height: 480,
    }
}

fn screenshot(controller: &ComputerController) -> String {
    match controller
        .execute(&ComputerRequest::ComputerScreenshot { window_id: 42 })
        .expect("screenshot")
    {
        ComputerOutput::ComputerScreenshot {
            observation_id,
            data_base64,
            ..
        } => {
            assert!(data_base64.starts_with("iVBOR"));
            observation_id
        }
        _ => panic!("expected screenshot"),
    }
}

#[test]
fn c1_observation_is_window_bound_single_use_and_checks_coordinates() {
    let (controller, calls) = controller(Duration::from_secs(30));
    let observation_id = screenshot(&controller);
    let click = ComputerRequest::ComputerClick {
        window_id: 42,
        observation_id: observation_id.clone(),
        x: 25,
        y: 30,
        button: ComputerMouseButton::Left,
        click_count: 1,
    };
    assert!(matches!(
        controller.execute(&click).expect("click"),
        ComputerOutput::ComputerAction {
            action: ComputerActionName::Click,
            ..
        }
    ));
    assert_eq!(calls.lock().expect("calls").clicks, [(25, 30)]);
    assert!(matches!(
        controller.execute(&click),
        Err(ComputerError::FreshScreenshot)
    ));

    let outside = ComputerRequest::ComputerClick {
        window_id: 42,
        observation_id: screenshot(&controller),
        x: 640,
        y: 10,
        button: ComputerMouseButton::Left,
        click_count: 1,
    };
    assert!(matches!(
        controller.execute(&outside),
        Err(ComputerError::OutsideScreenshot)
    ));
}

#[test]
fn c2_new_capture_revokes_old_token_and_receipt_never_echoes_typed_text() {
    let (controller, calls) = controller(Duration::from_secs(30));
    let old = screenshot(&controller);
    let current = screenshot(&controller);
    assert!(matches!(
        controller.execute(&ComputerRequest::ComputerTypeText {
            window_id: 42,
            observation_id: old,
            text: "no".into(),
        }),
        Err(ComputerError::FreshScreenshot)
    ));
    let secret_text = "not-stored-in-output";
    let output = controller
        .execute(&ComputerRequest::ComputerTypeText {
            window_id: 42,
            observation_id: current,
            text: secret_text.into(),
        })
        .expect("type");
    assert_eq!(calls.lock().expect("calls").typed, [secret_text]);
    assert!(
        !serde_json::to_string(&output)
            .expect("json")
            .contains(secret_text)
    );
}

#[test]
fn c3_expired_observation_and_stop_fail_closed_until_rebound() {
    let (controller, calls) = controller(Duration::ZERO);
    let expired = screenshot(&controller);
    assert!(matches!(
        controller.execute(&ComputerRequest::ComputerClose {
            window_id: 42,
            observation_id: expired,
        }),
        Err(ComputerError::FreshScreenshot)
    ));
    assert!(matches!(
        controller
            .execute(&ComputerRequest::ComputerStop)
            .expect("stop"),
        ComputerOutput::ComputerStop { stopped: true, .. }
    ));
    controller
        .execute(&ComputerRequest::ComputerStop)
        .expect("idempotent stop");
    assert_eq!(calls.lock().expect("calls").stops, 1);
    assert!(matches!(
        controller.execute(&ComputerRequest::ComputerListWindows),
        Err(ComputerError::Stopped)
    ));
}
