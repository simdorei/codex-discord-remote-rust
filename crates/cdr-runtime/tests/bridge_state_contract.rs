use std::fs;

use cdr_runtime::bridge_state::{BridgeState, BridgeStateError, SavedThreadSettings};

#[test]
fn python_selected_thread_and_settings_shape_is_read_without_conversion() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.json");
    fs::write(
        &path,
        "\u{feff}{\"selected_thread_id\":\" thread-1 \",\"thread_settings\":{\"thread-1\":{\"model\":\"gpt-5.6-sol\",\"reasoning\":\"ultra\",\"speed\":\"fast\",\"future\":1}}}",
    )
    .unwrap();
    let state = BridgeState::new(path);

    assert_eq!(
        state.selected_thread_id().unwrap().as_deref(),
        Some("thread-1")
    );
    assert_eq!(
        state.thread_settings("thread-1").unwrap(),
        SavedThreadSettings {
            model: Some("gpt-5.6-sol".into()),
            reasoning: Some("ultra".into()),
            speed: Some("fast".into()),
        }
    );
}

#[test]
fn writes_preserve_unknown_python_fields_and_support_clear_or_partial_settings() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("nested/state.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, r#"{"future":{"keep":true}}"#).unwrap();
    let state = BridgeState::new(path.clone());

    state.set_selected_thread_id(Some("thread-2")).unwrap();
    state
        .remember_thread_settings("thread-2", Some("model-a"), None, Some("standard"))
        .unwrap();
    state
        .remember_thread_settings("thread-2", None, Some("high"), None)
        .unwrap();
    state.set_selected_thread_id(None).unwrap();

    let value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(value["future"]["keep"], true);
    assert!(value.get("selected_thread_id").is_none());
    assert_eq!(value["thread_settings"]["thread-2"]["model"], "model-a");
    assert_eq!(value["thread_settings"]["thread-2"]["reasoning"], "high");
    assert_eq!(value["thread_settings"]["thread-2"]["speed"], "standard");
}

#[test]
fn app_server_fork_retargets_selection_and_copies_settings_without_erasing_source() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.json");
    fs::write(
        &path,
        r#"{"selected_thread_id":"source","thread_settings":{"source":{"model":"gpt-5.6-sol","reasoning":"max","speed":"fast","future":7}},"future":{"keep":true}}"#,
    )
    .unwrap();
    let state = BridgeState::new(path.clone());

    state.apply_thread_fork("source", "fork").unwrap();

    assert_eq!(state.selected_thread_id().unwrap().as_deref(), Some("fork"));
    assert_eq!(
        state.thread_settings("fork").unwrap(),
        SavedThreadSettings {
            model: Some("gpt-5.6-sol".into()),
            reasoning: Some("max".into()),
            speed: Some("fast".into()),
        }
    );
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(value["thread_settings"]["source"]["future"], 7);
    assert_eq!(value["thread_settings"]["fork"]["future"], 7);
    assert_eq!(value["future"]["keep"], true);
}

#[test]
fn app_server_fork_preserves_existing_target_settings_and_unrelated_selection() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.json");
    fs::write(
        &path,
        r#"{"selected_thread_id":"other","thread_settings":{"source":{"model":"source-model"},"fork":{"model":"target-model"}}}"#,
    )
    .unwrap();
    let state = BridgeState::new(path);

    state.apply_thread_fork("source", "fork").unwrap();

    assert_eq!(
        state.selected_thread_id().unwrap().as_deref(),
        Some("other")
    );
    assert_eq!(
        state.thread_settings("fork").unwrap().model.as_deref(),
        Some("target-model")
    );
}

#[test]
fn corrupt_state_surfaces_the_error_and_is_left_untouched() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.json");
    let corrupt = b"{not-json";
    fs::write(&path, corrupt).unwrap();
    let state = BridgeState::new(path.clone());

    assert!(matches!(
        state.selected_thread_id(),
        Err(BridgeStateError::Json { .. })
    ));
    assert_eq!(fs::read(path).unwrap(), corrupt);
}

#[test]
fn non_object_state_is_rejected_instead_of_silently_reset() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.json");
    fs::write(&path, "[]").unwrap();

    assert!(matches!(
        BridgeState::new(path).selected_thread_id(),
        Err(BridgeStateError::NotObject(_))
    ));
}
