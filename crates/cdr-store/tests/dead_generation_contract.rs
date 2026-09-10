use cdr_store::dead_generation::{
    DeadGenerationCapture, activate_runtime, capture_dead_generation, generation_is_sealed,
    target_is_held,
};
use cdr_store::queue::{NewQueueJob, QueueJobState, enqueue, list, try_begin_attempt};
use rusqlite::Connection;
use std::path::Path;

fn job<'a>(id: &'a str, target: &'a str) -> NewQueueJob<'a> {
    NewQueueJob {
        job_id: id,
        target_thread_id: target,
        channel_id: 10,
        owner_user_id: Some(20),
        discord_message_id: None,
        app_server_generation: 1,
        prompt: "preserve this request",
        queued: true,
        ack_sent: true,
        created_at: 1.0,
    }
}

fn capture(db: &Path, snapshot: &str, targets: &[String]) -> cdr_store::Result<bool> {
    capture_dead_generation(
        db,
        DeadGenerationCapture {
            runtime_id: "runtime-a",
            generation: 1,
            snapshot_json: snapshot,
            affected_targets: targets,
            startup_channel_id: Some(88),
            has_unscoped_requests: false,
            now: 2.0,
        },
    )
}

fn count(db: &Path, table: &str) -> i64 {
    Connection::open(db)
        .unwrap()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

// DG2: incident evidence and the notification are one transaction, and the
// original work remains byte-for-byte unchanged for a human to resolve.
#[test]
fn capture_preserves_full_evidence_and_routes_safe_notices() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    activate_runtime(&db, "runtime-a").unwrap();
    enqueue(&db, job("starting", "queue-thread")).unwrap();
    try_begin_attempt(&db, "starting", &["baseline".into()], 1)
        .unwrap()
        .unwrap();
    enqueue(&db, job("pending", "queue-thread")).unwrap();
    cdr_store::mapping::upsert_thread(&db, "request-thread", "project", "title", 98, 99, 1.0)
        .unwrap();
    let before = list(&db).unwrap();
    let snapshot = r#"{"generation":1,"closedReason":"lost","activeTurns":[{"threadId":"fallback-thread","turnId":"old-turn"}],"serverRequests":[{"id":"private-id","occurrence":"private-occurrence","method":"item/tool/requestUserInput","params":{"threadId":"request-thread","secretFixture":"do-not-emit"}}]}"#;
    assert!(
        capture(
            &db,
            snapshot,
            &["request-thread".into(), "fallback-thread".into()]
        )
        .unwrap()
    );
    assert_eq!(list(&db).unwrap(), before);
    for target in ["queue-thread", "request-thread", "fallback-thread"] {
        assert!(target_is_held(&db, target).unwrap());
    }
    let connection = Connection::open(&db).unwrap();
    let (saved, jobs): (String, String) = connection
        .query_row(
            "SELECT snapshot_json, queue_jobs_json FROM codex_dead_generation_incidents",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(saved, snapshot);
    let jobs: serde_json::Value = serde_json::from_str(&jobs).unwrap();
    assert_eq!(jobs.as_array().unwrap().len(), 1);
    assert_eq!(jobs[0]["job_id"], "starting");
    assert_eq!(jobs[0]["prompt"], "preserve this request");
    let notices = cdr_store::delivery::list_pending(&db).unwrap();
    assert_eq!(notices.len(), 3);
    for (target, channel) in [
        ("queue-thread", 10),
        ("request-thread", 99),
        ("fallback-thread", 88),
    ] {
        let notice = notices
            .iter()
            .find(|notice| notice.target_thread_id == target)
            .unwrap();
        assert_eq!(notice.channel_id, channel);
        assert!(notice.job_id.starts_with("dead-generation:"));
        assert!(!notice.content.contains("do-not-emit"));
        assert!(!notice.content.contains("private-id"));
        assert!(!notice.content.contains("preserve this request"));
    }
}

// DG3: deleting a successfully sent outbox row never deletes its incident receipt.
#[test]
fn recapture_after_notice_delivery_does_not_restage_and_changed_snapshot_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    activate_runtime(&db, "runtime-a").unwrap();
    let targets = ["held-thread".into()];
    capture(&db, "{\"generation\":1}", &targets).unwrap();
    for notice in cdr_store::delivery::list_pending(&db).unwrap() {
        cdr_store::delivery::complete(&db, &notice.delivery_id).unwrap();
    }
    assert!(!capture(&db, "{\"generation\":1}", &targets).unwrap());
    assert!(cdr_store::delivery::list_pending(&db).unwrap().is_empty());
    assert!(capture(&db, "{\"generation\":1,\"changed\":true}", &targets).is_err());
    assert_eq!(count(&db, "codex_dead_generation_incidents"), 1);
}

// DG4: a failed notification insert must roll back the incident and every hold.
#[test]
fn failed_outbox_transaction_does_not_seal_generation_or_change_work() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    activate_runtime(&db, "runtime-a").unwrap();
    enqueue(&db, job("starting", "held-thread")).unwrap();
    try_begin_attempt(&db, "starting", &[], 1).unwrap().unwrap();
    let before = list(&db).unwrap();
    Connection::open(&db)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_dead_notice BEFORE INSERT ON codex_delivery_outbox
         BEGIN SELECT RAISE(ABORT, 'fixture outbox failure'); END;",
        )
        .unwrap();
    assert!(capture(&db, "{}", &[]).is_err());
    assert_eq!(list(&db).unwrap(), before);
    assert_eq!(count(&db, "codex_dead_generation_incidents"), 0);
    assert_eq!(count(&db, "codex_dead_generation_holds"), 0);
    assert!(!generation_is_sealed(&db, 1).unwrap());
}

// DG5: even a dead process with no in-memory turn/request snapshot must capture
// Starting jobs and close the stale worker's late-attempt window.
#[test]
fn empty_snapshot_holds_starting_work_and_seals_only_this_runtime_generation() {
    use cdr_store::queue::{adopt_generation, adopt_target_generation};
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    activate_runtime(&db, "runtime-a").unwrap();
    enqueue(&db, job("starting", "held-thread")).unwrap();
    try_begin_attempt(&db, "starting", &[], 1).unwrap().unwrap();
    capture(&db, "{\"activeTurns\":[],\"serverRequests\":[]}", &[]).unwrap();
    enqueue(&db, job("late", "independent")).unwrap();
    assert!(try_begin_attempt(&db, "late", &[], 1).unwrap().is_none());
    assert_eq!(adopt_generation(&db, 2).unwrap().adopted_count, 1);
    assert_eq!(
        adopt_target_generation(&db, "held-thread", 2)
            .unwrap()
            .adopted_count,
        0
    );
    assert!(try_begin_attempt(&db, "late", &[], 2).unwrap().is_some());
    activate_runtime(&db, "runtime-b").unwrap();
    enqueue(&db, job("new-runtime", "new-thread")).unwrap();
    assert!(!generation_is_sealed(&db, 1).unwrap());
    assert!(
        try_begin_attempt(&db, "new-runtime", &[], 1)
            .unwrap()
            .is_some()
    );
    assert!(target_is_held(&db, "held-thread").unwrap());
}

// DG1: an existing durable hold must beat a stale queue worker's attempt CAS.
#[test]
fn held_target_cannot_claim_an_attempt_but_independent_target_can() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    enqueue(&db, job("held", "held-thread")).unwrap();
    enqueue(&db, job("independent", "other-thread")).unwrap();
    let connection = Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS codex_dead_generation_holds (
            target_thread_id TEXT PRIMARY KEY, runtime_id TEXT NOT NULL,
            generation INTEGER NOT NULL, created_at REAL NOT NULL);
         INSERT INTO codex_dead_generation_holds VALUES ('held-thread', 'runtime-a', 1, 2.0);",
        )
        .unwrap();

    assert!(
        try_begin_attempt(&db, "held", &[], 1).unwrap().is_none(),
        "a persisted dead-generation hold must block even a stale attempt claim"
    );
    assert!(
        try_begin_attempt(&db, "independent", &[], 1)
            .unwrap()
            .is_some()
    );
    let held = list(&db)
        .unwrap()
        .into_iter()
        .find(|row| row.job_id == "held")
        .unwrap();
    assert_eq!(held.state, QueueJobState::Pending);
    assert_eq!(held.attempt_count, 0);
    assert_eq!(held.prompt, "preserve this request");
}
