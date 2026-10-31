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
async fn async_question_shapes_preserve_context_and_render_only_valid_answers() {
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
        (json!(null), false),
        (json!([{"title":"자유 입력?","options":null}]), true),
        (
            json!([{"title":"잘못된 선택지?","options":[{"label":"예"}]}]),
            false,
        ),
        (
            json!([{"title":"너무 많은 선택지?","options":(0..26).map(|i|format!("선택 {i}")).collect::<Vec<_>>()}]),
            false,
        ),
        (json!([{"title":"선택지 생략"}]), true),
        (json!([{"title":"빈 선택지","options":[]}]), true),
        (json!([{"title":"빈 라벨","options":[""]}]), false),
        (json!([{"title":"빈 원문도 입력 가능","options":[]}]), true),
    ];
    for (i, (questions, supported)) in shapes.into_iter().enumerate() {
        let id = format!("shape-{i}");
        let text = if i == 7 {
            String::new()
        } else {
            format!("원문 보존 조건 {i}")
        };
        let params = json!({"threadId":"thread-b","turnId":"original","item":{"id":id,"type":"agentMessage","delivery":"async","phase":"final_answer","text":text,"questions":questions}});
        for _ in 0..2 {
            observe(db, server.instance_id(), 1, &params).unwrap();
            deliver_pending(db, server.instance_id(), 1, &http)
                .await
                .unwrap();
        }
        let key = store::occurrence_id("thread-b", "original", &id, 0).unwrap();
        assert_eq!(
            store::get(db, &key).unwrap().state,
            if supported { "open" } else { "unsupported" }
        );
    }
    server.close().await.unwrap();
    remote.stop.send(()).unwrap();
    let traffic = remote.task.await.unwrap();
    for i in 0..7 {
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
    let inputs: Vec<_> = traffic
        .iter()
        .flat_map(|(_, b)| {
            b["components"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|row| row["components"].as_array().into_iter().flatten())
        })
        .collect();
    assert_eq!(inputs.len(), 4);
    assert!(inputs.iter().all(|input| {
        input["label"] == "답변 입력"
            && input["custom_id"]
                .as_str()
                .unwrap()
                .starts_with("codex_async_other:")
    }));
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
