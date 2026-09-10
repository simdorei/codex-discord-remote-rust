use super::*;
use crate::test_support::{http_gate, message_fixture::MessageFixture};

fn message(id: u64, content: &str) -> Message {
    serde_json::from_value(serde_json::json!({
        "attachments":[],"author":{"avatar":null,"bot":false,"discriminator":"0001","id":"3","username":"fixture"},
        "channel_id":"42","content":content,"edited_timestamp":null,"embeds":[],"id":id.to_string(),
        "mention_everyone":false,"mention_roles":[],"mentions":[],"pinned":false,
        "timestamp":"2020-02-02T02:02:02.020000+00:00","tts":false,"type":0
    })).unwrap()
}

#[tokio::test]
async fn history_settings_rejection_reports_once_and_preserves_frozen_non_execution() {
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
    let mut io = DiscordHistoryCycleIo::new(
        fixture.context(root.path()),
        InteractionAccessPolicy {
            allow_all_channels: true,
            ..Default::default()
        },
        None,
        SystemTime::now(),
    );
    let raw = "!settings missing-target --model model-b";
    let adapted = io.adapt(message(910, raw)).unwrap();
    let HistoryPollItem::Candidate(candidate) = adapted.kind else {
        panic!("rejection remains a reportable candidate")
    };
    let HistoryClaimOutcome::Won(admitted) =
        io.claim(candidate, HistoryClaimPurpose::Process).unwrap()
    else {
        panic!("original admission")
    };
    let db = fixture.executor.mirror_db();
    let original = cdr_store::ingress::get(db, "message:910").unwrap().unwrap();
    assert_eq!(original.payload["content"], raw);
    assert!(
        original.payload["plan"]["Respond"]
            .as_str()
            .unwrap()
            .contains("missing-target")
    );
    // A later mapping cannot turn the frozen rejection into a settings action.
    cdr_store::mapping::upsert_thread(db, "thread-b", "p", "b", 100, 43, 2.0).unwrap();
    cdr_store::mapping::upsert_thread(db, "thread-a", "p", "a", 100, 42, 2.0).unwrap();
    gate.release.send(()).unwrap();
    io.process(admitted).await.unwrap();
    let replay = io.adapt(message(910, raw)).unwrap();
    let HistoryPollItem::Candidate(candidate) = replay.kind else {
        panic!("candidate")
    };
    assert!(matches!(
        io.claim(candidate, HistoryClaimPurpose::Process).unwrap(),
        HistoryClaimOutcome::Lost
    ));
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    assert_eq!(posts.len(), 1);
    assert!(posts[0]["content"].as_str().unwrap().starts_with("ERROR:"));
    let final_record = cdr_store::ingress::get(db, "message:910").unwrap().unwrap();
    assert_eq!(final_record.state, "completed");
    assert_eq!(final_record.payload, original.payload);
    assert!(cdr_store::queue::list(db).unwrap().is_empty());
    fixture.server.close().await.unwrap();
    let calls = std::fs::read_to_string(root.path().join("rpc.jsonl")).unwrap();
    assert!(
        !calls.contains("thread/resume")
            && !calls.contains("thread/settings/update")
            && !calls.contains("turn/start")
    );
}
