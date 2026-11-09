use super::*;
use cdr_remote_protocol::output::{
    ComputerActionName, ComputerScreenshotMediaType, ComputerWindowEntry,
};

fn fixture() -> (tempfile::TempDir, Coordinator, Coordinator) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("desktop.epoch");
    let name = format!("{GATE_NAME}-test-{}", Uuid::new_v4().simple());
    let make = || Coordinator {
        epoch_path: Some(path.clone()),
        gate_name: name.clone(),
        observation: None,
    };
    (directory, make(), make())
}

fn snapshot(id: &str) -> ComputerOutput {
    ComputerOutput::ComputerScreenshot {
        observation_id: id.into(),
        window: ComputerWindowEntry {
            window_id: 7,
            title: "test window".into(),
            process_name: "test.exe".into(),
            left: 0,
            top: 0,
            width: 100,
            height: 100,
            active: true,
        },
        media_type: ComputerScreenshotMediaType::Png,
        data_base64: "AAAAAAAAAAAA".into(),
    }
}

fn capture(coordinator: &mut Coordinator, id: &str) {
    coordinator
        .execute(
            &ComputerRequest::ComputerScreenshot { window_id: 7 },
            |_| Ok(snapshot(id)),
        )
        .unwrap();
}

fn input(id: &str) -> ComputerRequest {
    ComputerRequest::ComputerTypeText {
        window_id: 7,
        observation_id: id.into(),
        text: "test".into(),
    }
}

fn applied() -> ComputerOutput {
    ComputerOutput::ComputerAction {
        action: ComputerActionName::TypeText,
        window_id: Some(7),
        message: "typed".into(),
    }
}

#[test]
fn another_session_input_invalidates_an_unchanged_window_snapshot() {
    let (_directory, mut first, mut second) = fixture();
    capture(&mut first, "first-observation");
    capture(&mut second, "second-observation");
    second
        .execute(&input("second-observation"), |_| Ok(applied()))
        .unwrap();
    assert_eq!(
        first.execute(&input("first-observation"), |_| panic!(
            "stale input executed"
        )),
        Err(ComputerError::FreshScreenshot)
    );
    assert!(first.observation.is_none());
}

#[test]
fn another_session_listing_does_not_invalidate_a_snapshot() {
    let (_directory, mut first, mut second) = fixture();
    capture(&mut first, "first-observation");
    let before = fs::read(first.epoch_path().unwrap()).unwrap();
    second
        .execute(&ComputerRequest::ComputerListWindows, |_| {
            Ok(ComputerOutput::ComputerWindows { windows: vec![] })
        })
        .unwrap();
    assert_eq!(fs::read(first.epoch_path().unwrap()).unwrap(), before);
    first
        .execute(&input("first-observation"), |_| Ok(applied()))
        .unwrap();
}

#[test]
fn failed_native_input_still_invalidates_both_sessions_snapshots() {
    let (_directory, mut first, mut second) = fixture();
    capture(&mut first, "first-observation");
    capture(&mut second, "second-observation");
    assert!(
        second
            .execute(&input("second-observation"), |_| {
                Err(ComputerError::Platform("partially applied input".into()))
            })
            .is_err()
    );
    for (coordinator, id) in [
        (&mut first, "first-observation"),
        (&mut second, "second-observation"),
    ] {
        assert_eq!(
            coordinator.execute(&input(id), |_| panic!("replayed input executed")),
            Err(ComputerError::FreshScreenshot)
        );
    }
}

#[test]
fn unsuccessful_or_mismatched_capture_drops_the_previous_observation() {
    let (_directory, mut first, _second) = fixture();
    capture(&mut first, "first-observation");
    assert!(
        first
            .execute(
                &ComputerRequest::ComputerScreenshot { window_id: 7 },
                |_| { Err(ComputerError::Platform("capture failed".into())) }
            )
            .is_err()
    );
    assert_eq!(
        first.execute(&input("first-observation"), |_| panic!(
            "old input executed"
        )),
        Err(ComputerError::FreshScreenshot)
    );
    assert_eq!(
        first.execute(
            &ComputerRequest::ComputerScreenshot { window_id: 8 },
            |_| { Ok(snapshot("wrong-window-observation")) }
        ),
        Err(ComputerError::DifferentWindow)
    );
    assert!(first.observation.is_none());
}

#[test]
fn missing_or_corrupted_state_never_authorizes_an_existing_observation() {
    let (_directory, mut first, _second) = fixture();
    capture(&mut first, "deleted-state-observation");
    fs::remove_file(first.epoch_path().unwrap()).unwrap();
    assert_eq!(
        first.execute(&input("deleted-state-observation"), |_| panic!(
            "stale input executed"
        )),
        Err(ComputerError::FreshScreenshot)
    );
    for contents in [
        b"invalid".as_slice(),
        &[b'x'; 4096],
        b"00000000000000000000000000000000",
    ] {
        capture(&mut first, "corrupt-state-observation");
        fs::write(first.epoch_path().unwrap(), contents).unwrap();
        assert!(
            first
                .execute(&input("corrupt-state-observation"), |_| {
                    panic!("corrupted state authorized input")
                })
                .is_err()
        );
        assert_eq!(fs::read(first.epoch_path().unwrap()).unwrap(), contents);
        // Stop can publish a new revision even if the previous file was invalid.
        first
            .execute(&ComputerRequest::ComputerStop, |_| {
                Ok(ComputerOutput::ComputerStop {
                    stopped: true,
                    message: "stopped".into(),
                })
            })
            .unwrap();
    }
}

#[test]
fn failure_to_publish_blocks_the_native_action() {
    let (directory, mut first, _second) = fixture();
    let blocked = directory.path().join("occupied-parent");
    fs::write(&blocked, b"fixture").unwrap();
    first.epoch_path = Some(blocked.join("desktop.epoch"));
    assert!(
        first
            .execute(&ComputerRequest::ComputerActivate { window_id: 7 }, |_| {
                panic!("input ran before revision could be published")
            })
            .is_err()
    );
}

#[test]
fn constructing_a_coordinator_does_not_create_coordination_state() {
    let (_directory, first, _second) = fixture();
    assert!(!first.epoch_path().unwrap().exists());
    assert!(first.observation.is_none());
}

#[test]
fn absent_or_relative_coordination_path_never_reaches_the_native_backend() {
    let (_directory, mut first, _second) = fixture();
    for invalid in [None, Some(PathBuf::from("relative/desktop.epoch"))] {
        first.epoch_path = invalid;
        assert!(
            first
                .execute(&ComputerRequest::ComputerListWindows, |_| {
                    panic!("unconfigured computer operation executed")
                })
                .is_err()
        );
    }
}

#[test]
fn listing_does_not_create_the_lazy_user_directory() {
    let (directory, mut first, _second) = fixture();
    let lazy_parent = directory.path().join("lazy-computer-use");
    first.epoch_path = Some(lazy_parent.join(EPOCH_FILE));
    first
        .execute(&ComputerRequest::ComputerListWindows, |_| {
            Ok(ComputerOutput::ComputerWindows { windows: vec![] })
        })
        .unwrap();
    assert!(!lazy_parent.exists());
    capture(&mut first, "lazy-directory-observation");
    assert!(first.epoch_path().unwrap().is_file());
}
