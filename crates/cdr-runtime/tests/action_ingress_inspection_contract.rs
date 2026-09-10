use std::collections::BTreeSet;
use std::sync::Arc;

use cdr_runtime::action_executor::ActionExecutor;
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_runtime::message_plan::{IncomingMessage, MessagePlan, plan_message};
use cdr_runtime::prefix_plan::plan_prefix;
use cdr_store::ingress::{IngressKind, NewIngress, StoredIngress, admit, get, hold};
use serde_json::{Value, json};

#[path = "support/action_target.rs"]
mod action_target;
use action_target::{FakeBackend, executor};

#[path = "support/ingress_display_preservation.rs"]
mod preservation_contract;

#[test]
fn runners_request_id_routes_to_exact_read_only_inspection() {
    assert_eq!(
        serde_json::to_value(plan_message(&input("!runners action:owned")).unwrap()).unwrap(),
        json!({"Execute":{"SavedRequest":{"request_id":"action:owned"}}})
    );
    assert!(plan_prefix("runners action:owned extra").is_err());
    assert!(plan_prefix("runners release action:owned").is_err());
    assert_eq!(
        serde_json::to_value(plan_message(&input("!runners")).unwrap()).unwrap(),
        json!({"Execute":"Runners"})
    );
}

#[tokio::test]
async fn runners_lists_only_actor_and_channel_holds_without_original_payload() {
    let (_temp, executor, backend) = fixture();
    seed(
        &executor,
        "action:owned",
        99,
        20,
        json!({"prompt":"private original"}),
    );
    seed(
        &executor,
        "action:other-user",
        99,
        21,
        json!({"prompt":"other user data"}),
    );
    seed(
        &executor,
        "action:other-channel",
        100,
        20,
        json!({"prompt":"other channel data"}),
    );

    let result = executor
        .execute(CommandAction::Runners, 99, 20)
        .await
        .unwrap();

    assert!(result.text.contains("pending: 0"));
    assert!(
        result.text.contains("action:owned"),
        "a held request must be visible to its owner"
    );
    assert!(result.text.contains("saved-target"));
    assert!(result.text.contains("fixture interrupted"));
    assert!(result.text.contains("!runners <request_id>"));
    assert!(!result.text.contains("action:other-user"));
    assert!(!result.text.contains("action:other-channel"));
    assert!(!result.text.contains("private original"));
    assert!(!result.text.contains("other user data"));
    assert!(backend.starts.lock().await.is_empty());
}

#[tokio::test]
async fn original_owner_in_original_channel_can_read_exact_payload_without_mutation() {
    let (_temp, executor, backend) = fixture();
    let payload = json!({
        "prompt":"  보존할 원문\nexact spacing and <@123> remain\n",
        "attachments":[{"filename":"notes.txt","url":"https://example.invalid/notes.txt"}],
        "options":{"empty":"","nullable":null,"flags":[true,false]}
    });
    let before = seed(&executor, "action:owned", 99, 20, payload.clone());
    let result = executor
        .execute(detail_action("action:owned"), 99, 20)
        .await
        .unwrap();

    assert_eq!(display_payload(&result.text), payload);
    assert!(result.text.contains("action:owned"));
    assert!(result.text.contains("saved-target"));
    assert!(!result.waits_for_final);
    assert!(result.ui.is_none());
    assert_eq!(
        get(executor.mirror_db(), "action:owned").unwrap(),
        Some(before)
    );
    assert!(
        cdr_store::queue::list(executor.mirror_db())
            .unwrap()
            .is_empty()
    );
    assert!(backend.starts.lock().await.is_empty());
    assert!(backend.forks.lock().await.is_empty());
    assert!(backend.resumes.lock().await.is_empty());
}

#[tokio::test]
async fn different_owner_or_channel_cannot_read_or_learn_whether_request_exists() {
    let (_temp, executor, backend) = fixture();
    seed(
        &executor,
        "action:owned",
        99,
        20,
        json!({"prompt":"private original"}),
    );
    for (channel, user) in [(99, 21), (100, 20), (100, 21)] {
        let forbidden = executor
            .execute(detail_action("action:owned"), channel, user)
            .await;
        let missing = executor
            .execute(detail_action("action:missing"), channel, user)
            .await;
        assert!(
            forbidden.is_err(),
            "saved payload inspection must check both owner and channel"
        );
        assert_eq!(
            forbidden.unwrap_err().to_string(),
            missing.unwrap_err().to_string()
        );
    }
    assert!(backend.starts.lock().await.is_empty());
}

#[tokio::test]
async fn explicit_credential_fields_are_redacted_only_in_the_authorized_display_copy() {
    let (_temp, executor, _) = fixture();
    let original = json!({
        "prompt":"ordinary raw prompt stays unchanged",
        "authorization":"fixture-secret-authorization",
        "metadata":{"Token":"fixture-secret-token","description":"retained"},
        "attachments":[{"api_key":"fixture-secret-key","filename":"notes.txt"}]
    });
    let before = seed(&executor, "action:owned", 99, 20, original);
    let result = executor
        .execute(detail_action("action:owned"), 99, 20)
        .await
        .unwrap();

    assert_eq!(
        display_payload(&result.text),
        json!({
            "prompt":"ordinary raw prompt stays unchanged",
            "metadata":{"description":"retained"},
            "attachments":[{"filename":"notes.txt"}]
        })
    );
    assert!(!result.text.contains("fixture-secret"));
    assert_eq!(
        get(executor.mirror_db(), "action:owned").unwrap(),
        Some(before)
    );
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

fn seed(
    executor: &ActionExecutor<FakeBackend>,
    key: &str,
    channel: i64,
    user: i64,
    payload: Value,
) -> StoredIngress {
    admit(
        executor.mirror_db(),
        &NewIngress {
            ingress_id: key.into(),
            kind: IngressKind::Action,
            event_id: None,
            application_id: None,
            channel_id: channel,
            owner_user_id: user,
            source_message_id: None,
            payload,
            target_thread_id: Some("saved-target".into()),
            canonical_owner: None,
            now: 1.0,
        },
    )
    .unwrap();
    hold(executor.mirror_db(), key, "fixture interrupted", false, 2.0).unwrap();
    get(executor.mirror_db(), key).unwrap().unwrap()
}

fn detail_action(request_id: &str) -> CommandAction {
    let command = format!("!runners {request_id}");
    match plan_message(&input(&command)).unwrap() {
        MessagePlan::Execute(action) => action,
        other => panic!("inspection must remain a command: {other:?}"),
    }
}

fn display_payload(text: &str) -> Value {
    let (_, payload) = text
        .split_once("original_payload:\n")
        .expect("inspection must expose the saved original payload to its authorized owner");
    serde_json::from_str(payload).unwrap()
}

fn input(content: &str) -> IncomingMessage<'_> {
    IncomingMessage {
        content,
        message_content_enabled: true,
        channel_allowed: true,
        user_allowed: true,
        author_is_bot: false,
        author_is_self: false,
        author_mentions_bridge: false,
        has_attachments: false,
        mirrored_target: false,
        mentioned_user_ids: BTreeSet::new(),
        required_plain_ask_user_ids: BTreeSet::new(),
    }
}
