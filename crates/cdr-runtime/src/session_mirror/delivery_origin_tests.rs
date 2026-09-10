use super::*;
use crate::session_mirror::{MirrorDetail, collect_items};
use serde_json::json;

#[test]
fn queued_same_text_is_not_evidence_that_an_app_user_message_came_from_discord() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    cdr_store::queue::enqueue(
        &db,
        cdr_store::queue::NewQueueJob {
            job_id: "queued",
            target_thread_id: "thread",
            channel_id: 42,
            owner_user_id: Some(43),
            discord_message_id: Some(44),
            app_server_generation: 1,
            prompt: "same request",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    let events=[json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"app-turn"}}),
        json!({"timestamp":"2","type":"event_msg","payload":{"type":"user_message","message":"same request"}})]
        .into_iter().map(|v|v.as_object().unwrap().clone()).collect::<Vec<_>>();
    let item = collect_items("thread", &events, MirrorDetail::Send).remove(0);
    let jobs = cdr_store::queue::list(&db).unwrap();
    assert!(
        !discord_origin_user(&db, "thread", &item, &jobs).unwrap(),
        "an unstarted queued prompt cannot own an already observed app turn"
    );
}
