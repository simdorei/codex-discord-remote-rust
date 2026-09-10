use std::sync::Arc;

use cdr_runtime::action_executor::ActionExecutor;
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_store::ingress::{
    IngressKind, NewIngress, StoredIngress, admit, confirm, get, hold, record_result,
};
use serde_json::json;

#[path = "support/action_target.rs"]
mod action_target;
use action_target::{FakeBackend, executor};

#[tokio::test]
async fn runners_shows_all_latest_twenty_mixed_attention_states_without_mutation() {
    let (_temp, executor, backend) = fixture();
    let mut before = vec![
        seed(&executor, "action:old-hold", 99, 20, 1.0, State::Held),
        seed(&executor, "action:boundary-hold", 99, 20, 2.0, State::Held),
    ];
    for index in 0..19 {
        before.push(seed(
            &executor,
            &format!("action:completed-{index:02}"),
            99,
            20,
            f64::from(index + 3),
            State::Completed,
        ));
    }
    // These newer rows must neither appear nor consume the owner's 20 slots.
    for (key, channel, user, created_at, state) in [
        ("action:other-user", 99, 21, 100.0, State::Completed),
        ("action:other-channel", 100, 20, 101.0, State::Completed),
        ("action:already-confirmed", 99, 20, 102.0, State::Confirmed),
        ("action:not-executed", 99, 20, 103.0, State::Staged),
    ] {
        before.push(seed(&executor, key, channel, user, created_at, state));
    }

    let result = executor
        .execute(CommandAction::Runners, 99, 20)
        .await
        .unwrap();
    let rows: Vec<_> = result
        .text
        .lines()
        .filter(|line| line.starts_with("action:"))
        .collect();
    assert_eq!(
        rows.len(),
        20,
        "completed requests awaiting confirmation must remain visible"
    );
    let expected_ids: Vec<_> = (0..19)
        .rev()
        .map(|index| format!("action:completed-{index:02}"))
        .chain(std::iter::once("action:boundary-hold".into()))
        .collect();
    let actual_ids: Vec<_> = rows
        .iter()
        .map(|line| line.split(" | ").next().unwrap())
        .collect();
    assert_eq!(
        actual_ids, expected_ids,
        "the limit applies to the displayed mixed list"
    );
    for row in &rows[..19] {
        assert!(row.contains("status: execution completed; confirmation pending"));
        assert!(row.contains("reason: confirmation delivery not recorded"));
        assert!(!row.contains("manual review required"));
    }
    assert!(rows[19].contains("status: saved; manual review required"));
    assert!(rows[19].contains("reason: fixture interrupted"));
    assert!(result.text.contains("latest 20"));
    assert!(result.text.contains("!runners <request_id>"));
    assert!(!result.text.contains("private listing payload"));
    assert!(!result.text.contains("private execution result"));
    for record in before {
        assert_eq!(
            get(executor.mirror_db(), &record.ingress_id).unwrap(),
            Some(record)
        );
    }
    assert!(
        cdr_store::queue::list(executor.mirror_db())
            .unwrap()
            .is_empty()
    );
    assert!(backend.starts.lock().await.is_empty());
    assert!(backend.forks.lock().await.is_empty());
    assert!(backend.resumes.lock().await.is_empty());
}

fn fixture() -> (
    tempfile::TempDir,
    ActionExecutor<FakeBackend>,
    Arc<FakeBackend>,
) {
    let temp = tempfile::tempdir().unwrap();
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    let backend = Arc::new(FakeBackend::default());
    let executor = executor(
        &temp,
        temp.path().join("mirror.sqlite"),
        bridge,
        backend.clone(),
    );
    (temp, executor, backend)
}

#[derive(Clone, Copy)]
enum State {
    Held,
    Completed,
    Confirmed,
    Staged,
}

fn seed(
    executor: &ActionExecutor<FakeBackend>,
    key: &str,
    channel: i64,
    user: i64,
    created_at: f64,
    state: State,
) -> StoredIngress {
    let path = executor.mirror_db();
    admit(
        path,
        &NewIngress {
            ingress_id: key.into(),
            kind: IngressKind::Action,
            event_id: None,
            application_id: None,
            channel_id: channel,
            owner_user_id: user,
            source_message_id: None,
            payload: json!({"prompt": "private listing payload"}),
            target_thread_id: Some("saved-target".into()),
            canonical_owner: None,
            now: created_at,
        },
    )
    .unwrap();
    match state {
        State::Held => hold(path, key, "fixture interrupted", false, created_at + 0.1).unwrap(),
        State::Completed | State::Confirmed => {
            record_result(
                path,
                key,
                &json!({"response": "private execution result"}),
                created_at + 0.1,
            )
            .unwrap();
            if matches!(state, State::Confirmed) {
                confirm(path, key, created_at + 0.2).unwrap();
            }
        }
        State::Staged => {}
    }
    get(path, key).unwrap().unwrap()
}
