use crate::{
    message_worker::process_admitted_gateway_message,
    test_support::{http_gate, message_fixture::MessageFixture},
};
use std::{sync::Arc, time::Duration};
#[path = "../../tests/support/settings_app_server.rs"]
mod app;

#[tokio::test]
async fn settings_prefix_rejects_mapping_change_after_admission() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(app::start(&temp, &log, "normal").await);
    let gate = http_gate::start().await;
    let fixture = MessageFixture::with_server(
        &temp,
        Arc::new(
            twilight_http::Client::builder()
                .proxy(gate.address, true)
                .ratelimiter(None)
                .build(),
        ),
        server.clone(),
    );
    let work = fixture.admit("!settings --model model-b");
    let db = fixture.executor.mirror_db();
    let original = cdr_store::ingress::get(db, "message:801").unwrap().unwrap();
    cdr_store::mapping::upsert_thread(db, "thread-b", "project", "b", 100, 43, 2.0).unwrap();
    cdr_store::mapping::upsert_thread(db, "thread-a", "project", "a", 100, 42, 2.0).unwrap();
    gate.release.send(()).unwrap();
    let result = process_admitted_gateway_message(work, &fixture.context(temp.path())).await;
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    server.close().await.unwrap();
    assert!(
        result.is_err(),
        "settings was silently redirected after admission"
    );
    assert!(posts.is_empty());
    let after = cdr_store::ingress::get(db, "message:801").unwrap().unwrap();
    assert_eq!(after.payload, original.payload);
    assert_eq!(after.target_thread_id, original.target_thread_id);
    let calls = std::fs::read_to_string(log).unwrap();
    assert!(!calls.contains("thread/settings/update") && !calls.contains("thread/resume"));
}

#[tokio::test]
async fn settings_option_query_never_discards_an_explicit_unknown_target() {
    let temp = tempfile::tempdir().unwrap();
    let server = Arc::new(app::start(&temp, &temp.path().join("rpc.jsonl"), "normal").await);
    let gate = http_gate::start().await;
    let fixture = MessageFixture::with_server(
        &temp,
        Arc::new(
            twilight_http::Client::builder()
                .proxy(gate.address, true)
                .ratelimiter(None)
                .build(),
        ),
        server.clone(),
    );
    gate.release.send(()).unwrap();
    let result = process_admitted_gateway_message(
        fixture.admit("!settings missing-target --model"),
        &fixture.context(temp.path()),
    )
    .await;
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    server.close().await.unwrap();
    assert!(
        result.is_err(),
        "explicit target was discarded by an options query"
    );
    assert!(posts.is_empty());
}

#[tokio::test]
async fn actual_settings_prefix_query_and_change_return_the_verified_value() {
    for (raw, change) in [
        ("!settings", false),
        ("!settings thread-b", false),
        ("!settings --model \"Model B\"", true),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("rpc.jsonl");
        let server = Arc::new(app::start(&temp, &log, "normal").await);
        server
            .request(
                "test/observe",
                serde_json::json!({"threadId":"thread-b"}),
                Duration::from_secs(2),
                None,
            )
            .await
            .unwrap();
        let gate = http_gate::start().await;
        let fixture = MessageFixture::with_server(
            &temp,
            Arc::new(
                twilight_http::Client::builder()
                    .proxy(gate.address, true)
                    .ratelimiter(None)
                    .build(),
            ),
            server.clone(),
        );
        gate.release.send(()).unwrap();
        process_admitted_gateway_message(fixture.admit(raw), &fixture.context(temp.path()))
            .await
            .unwrap();
        gate.stop.send(()).unwrap();
        let posts = gate.task.await.unwrap();
        assert_eq!(posts.len(), 1);
        let text = posts[0]["content"].as_str().unwrap();
        if change {
            assert_eq!(text, "모델이 변경되었습니다: model-b");
        } else {
            assert!(text.contains("마지막 서버 확인값") && text.contains("model-a"));
        }
        assert_eq!(
            cdr_store::ingress::get(fixture.executor.mirror_db(), "message:801")
                .unwrap()
                .unwrap()
                .state,
            "completed"
        );
        server.close().await.unwrap();
        let calls = std::fs::read_to_string(log).unwrap();
        assert_eq!(
            calls
                .lines()
                .filter(|line| line.contains("thread/settings/update"))
                .count(),
            usize::from(change)
        );
        if !change {
            assert!(!calls.contains("thread/resume"));
        }
        assert!(!calls.contains("turn/start") && !calls.contains("thread/fork"));
    }
}
