//! Regression: stop custody must survive lossless DB persistence of queue timestamps.
use cdr_store::{
    ingress::stop::{StopScope, control},
    queue::{NewQueueJob, begin_attempt, enqueue, list_filtered, mark_running},
};
use rusqlite::params;
use serde_json::{Value, json};

#[test]
fn persisted_stop_control_keeps_fractional_queue_evidence_and_claims_once() {
    let base = 1_790_584_100.0_f64;
    let timestamp = (1..=1024)
        .map(|offset| f64::from_bits(base.to_bits() + offset))
        .find(|value| {
            let original = serde_json::to_string(value).unwrap();
            let decoded: Value = serde_json::from_str(&original).unwrap();
            serde_json::to_string(&decoded).unwrap() != original
        })
        .unwrap_or(base);
    let encoded = serde_json::to_string(&timestamp).unwrap();
    let decoded: Value = serde_json::from_str(&encoded).unwrap();
    println!("timestamp evidence: original={encoded}; Value roundtrip={decoded}");
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("stop.sqlite");
    enqueue(
        &db,
        NewQueueJob {
            job_id: "original",
            target_thread_id: "thread-a",
            channel_id: 99,
            owner_user_id: Some(20),
            discord_message_id: Some(101),
            app_server_generation: 7,
            prompt: "original input",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    begin_attempt(&db, "original", &[], 7).unwrap();
    mark_running(&db, "original", "owned-a", 7).unwrap();
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute(
            "UPDATE codex_turn_queue SET created_at=?1,updated_at=?1 WHERE job_id='original'",
            params![timestamp],
        )
        .unwrap();
    let before =
        serde_json::to_string(&list_filtered(&db, Some("thread-a"), None).unwrap()).unwrap();
    let original=control::accept_running(&db,
        StopScope {target:"thread-a",channel:99,owner:20},
        &json!({"target":"thread-a","route":"Explicit","command":{"Stop":{"reference":"thread-a"}}}),
        None,("resident-a",7),||Ok(()),
    ).unwrap().unwrap();
    let expected = serde_json::to_string(&original).unwrap();
    let restored = control::pending_after(&db, 0).unwrap().pop().unwrap().1;
    assert_eq!(
        serde_json::to_string(&restored).unwrap(),
        expected,
        "durable stop authority must preserve the exact original queue evidence"
    );
    assert!(
        control::claim(&db, &restored, || Ok(())).unwrap().is_some(),
        "the unchanged original turn must retain its one-use interrupt authority"
    );
    assert_eq!(
        serde_json::to_string(&list_filtered(&db, Some("thread-a"), None).unwrap()).unwrap(),
        before
    );
    assert!(control::claim(&db, &restored, || Ok(())).unwrap().is_none());
}
