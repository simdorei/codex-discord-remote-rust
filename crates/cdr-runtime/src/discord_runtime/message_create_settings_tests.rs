use super::*;
use crate::test_support::{http_gate, message_fixture::MessageFixture};

#[tokio::test]
async fn gateway_settings_admission_error_is_durably_reported_not_an_internal_failure() {
    let root = tempfile::tempdir().unwrap();
    let gate = http_gate::start().await;
    let fixture = MessageFixture::new(
        &root,
        Arc::new(
            twilight_http::Client::builder()
                .proxy(gate.address, true)
                .ratelimiter(None)
                .build(),
        ),
    )
    .await;
    let input: Message = serde_json::from_value(serde_json::json!({
        "attachments":[],"author":{"avatar":null,"bot":false,"discriminator":"0001","id":"3","username":"fixture"},
        "channel_id":"42","content":"!settings missing-target --model model-b","edited_timestamp":null,"embeds":[],"id":"920",
        "mention_everyone":false,"mention_roles":[],"mentions":[],"pinned":false,
        "timestamp":"2020-02-02T02:02:02.020000+00:00","tts":false,"type":0
    })).unwrap();
    let context = fixture.context(root.path());
    let resolver = fixture.executor.settings_resolver();
    let db = fixture.executor.mirror_db();
    gate.release.send(()).unwrap();
    for _ in 0..2 {
        dispatch_message_create(
            ErrorReportTarget::from_message(&input),
            || {
                prepare_message_create(
                    input.clone(),
                    None,
                    context.config,
                    db,
                    &AdmissionGate::new(),
                    false,
                    &resolver,
                )
            },
            |admitted| process_admitted_gateway_message(admitted, &context),
            |_, _| async {
                panic!("expected rejection must use its frozen response, not internal error path")
            },
        )
        .await
        .unwrap();
    }
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    assert_eq!(posts.len(), 1);
    assert!(posts[0]["content"].as_str().unwrap().contains("ERROR:"));
    assert!(
        posts[0]["content"]
            .as_str()
            .unwrap()
            .contains("missing-target")
    );
    let row = cdr_store::ingress::get(db, "message:920").unwrap().unwrap();
    assert_eq!(row.state, "completed");
    assert_eq!(row.payload["content"], input.content);
    assert!(row.payload["plan"]["Respond"].is_string());
    assert!(cdr_store::queue::list(db).unwrap().is_empty());
    fixture.server.close().await.unwrap();
    let calls = std::fs::read_to_string(root.path().join("rpc.jsonl")).unwrap();
    assert!(
        !calls.contains("thread/resume")
            && !calls.contains("thread/settings/update")
            && !calls.contains("turn/start")
    );
}
