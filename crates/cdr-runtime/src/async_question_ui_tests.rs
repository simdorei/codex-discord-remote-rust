//! D2-shapes: inspect actual HTTP bodies, not just successful parser returns.
use super::*;
use crate::{
    soak::native_fixture,
    test_support::{approval_http, message_fixture::MessageFixture},
};
use cdr_app_server::ResidentAppServer;
use cdr_store::queue;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn async_question_unsupported_shapes_preserve_context_once_across_replay() {
    let temp = tempfile::tempdir().unwrap();
    let remote = approval_http::start().await;
    let http = Arc::new(
        Client::builder()
            .token("fixture-token".into())
            .proxy(remote.address.clone(), true)
            .ratelimiter(None)
            .build(),
    );
    let mut config = native_fixture::config("async-question");
    config.environment.insert(
        "CDR_ACTION_RPC_LOG".into(),
        temp.path().join("rpc.jsonl").to_string_lossy().into_owned(),
    );
    let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
    let f = MessageFixture::with_server(&temp, http.clone(), server.clone());
    let db = f.executor.mirror_db();
    queue::enqueue(
        db,
        queue::NewQueueJob {
            job_id: "origin",
            target_thread_id: "thread-b",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: Some(40),
            app_server_generation: 1,
            prompt: "question",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    queue::begin_attempt(db, "origin", &[], 1).unwrap();
    queue::mark_running(db, "origin", "original", 1).unwrap();
    let shapes = [
        json!(null),
        json!([{"title":"자유 입력?","options":null}]),
        json!([{"title":"잘못된 선택지?","options":[{"label":"예"}]}]),
        json!([{"title":"너무 많은 선택지?","options":(0..26).map(|i|format!("선택 {i}")).collect::<Vec<_>>()}]),
    ];
    for (i, questions) in shapes.into_iter().enumerate() {
        let id = format!("shape-{i}");
        let text = format!("원문 보존 조건 {i}");
        let params = json!({"threadId":"thread-b","turnId":"original","item":{"id":id,"type":"agentMessage","delivery":"async","phase":"final_answer","text":text,"questions":questions}});
        for _ in 0..2 {
            observe(db, server.instance_id(), 1, &params).unwrap();
            deliver_pending(db, server.instance_id(), 1, &http)
                .await
                .unwrap();
        }
        let key = store::occurrence_id("thread-b", "original", &id, 0).unwrap();
        assert_eq!(store::get(db, &key).unwrap().state, "unsupported");
    }
    server.close().await.unwrap();
    remote.stop.send(()).unwrap();
    let traffic = remote.task.await.unwrap();
    for i in 0..4 {
        assert_eq!(
            traffic
                .iter()
                .filter(|(_, b)| b["content"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("원문 보존 조건 {i}")))
                .count(),
            1
        );
    }
    assert_eq!(
        traffic
            .iter()
            .filter(|(_, b)| b["content"]
                .as_str()
                .unwrap()
                .contains("선택 버튼을 만들 수 없습니다"))
            .count(),
        4
    );
    assert!(
        traffic
            .iter()
            .all(|(_, b)| b["components"].as_array().is_none_or(Vec::is_empty))
    );
    assert!(
        traffic
            .iter()
            .all(|(_, b)| !b["content"].as_str().unwrap().starts_with("Final"))
    );
    assert!(
        traffic
            .iter()
            .any(|(_, b)| b["content"].as_str().unwrap().contains("질문 형식 오류"))
    );
}
