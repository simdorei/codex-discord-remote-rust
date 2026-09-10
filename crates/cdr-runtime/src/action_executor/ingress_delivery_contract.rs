use crate::{
    message_worker::process_admitted_gateway_message,
    test_support::{http_gate, message_fixture::MessageFixture},
};
use cdr_store::ingress::{IngressKind, NewIngress, admit, get, hold};
use serde_json::{Value, json};
use std::sync::Arc;

#[tokio::test]
async fn actual_saved_request_delivery_preserves_long_original_json_and_does_not_retry_it() {
    let temp = tempfile::tempdir().unwrap();
    let gate = http_gate::start().await;
    let fixture = MessageFixture::new(
        &temp,
        Arc::new(
            twilight_http::Client::builder()
                .proxy(gate.address, true)
                .ratelimiter(None)
                .build(),
        ),
    )
    .await;
    let prompt = "한  ".repeat(2500);
    let original = json!({"prompt":prompt,"attachments":[{"filename":"메모.txt","url":"https://example.invalid/note"}]});
    let db = fixture.executor.mirror_db();
    admit(
        db,
        &NewIngress {
            ingress_id: "action:large".into(),
            kind: IngressKind::Action,
            event_id: None,
            application_id: None,
            channel_id: 42,
            owner_user_id: 3,
            source_message_id: None,
            payload: original.clone(),
            target_thread_id: Some("thread-b".into()),
            canonical_owner: None,
            now: 1.0,
        },
    )
    .unwrap();
    hold(db, "action:large", "fixture interrupted", false, 2.0).unwrap();
    let before = get(db, "action:large").unwrap();
    gate.release.send(()).unwrap();
    process_admitted_gateway_message(
        fixture.admit("!runners action:large"),
        &fixture.context(temp.path()),
    )
    .await
    .unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    fixture.server.close().await.unwrap();
    assert!(posts.len() > 1);
    let mut text = String::new();
    for (index, post) in posts.iter().enumerate() {
        let content = post["content"].as_str().unwrap();
        assert!(content.chars().count() <= 1900);
        let marker = format!("[{}/{}]\n", index + 1, posts.len());
        text.push_str(content.strip_prefix(&marker).expect("ordered chunk marker"));
    }
    let (_, payload) = text.split_once("original_payload:").unwrap();
    let displayed: Value = serde_json::from_str(payload).unwrap();
    assert_eq!(
        displayed, original,
        "display chunks changed saved request text"
    );
    assert_eq!(get(db, "action:large").unwrap(), before);
    assert!(cdr_store::queue::list(db).unwrap().is_empty());
    let rpc = std::fs::read_to_string(temp.path().join("rpc.jsonl")).unwrap();
    assert!(!rpc.contains("turn/start") && !rpc.contains("thread/fork"));
}
