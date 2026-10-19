use cdr_store::{
    commentary_outbox,
    completion_work::{self, Cursor, Source},
    delivery, delivery_receipt, queue,
};
use rusqlite::params;
use std::path::Path;

const THREAD: &str = "same-thread";

fn seed(path: &Path, job: &str, at: f64, acknowledged: bool) {
    queue::enqueue(
        path,
        queue::NewQueueJob {
            job_id: job,
            target_thread_id: THREAD,
            channel_id: 42,
            owner_user_id: Some(1),
            discord_message_id: None,
            app_server_generation: 1,
            prompt: "fixture",
            queued: false,
            ack_sent: acknowledged,
            created_at: at,
        },
    )
    .unwrap();
    queue::begin_attempt(path, job, &[], 1).unwrap();
    queue::mark_running(path, job, &format!("turn-{job}"), 1).unwrap();
    commentary_outbox::stage(path, THREAD, &format!("turn-{job}"), job).unwrap();
    commentary_outbox::stage(
        path,
        THREAD,
        &format!("turn-{job}"),
        &format!("later-{job}"),
    )
    .unwrap();
    delivery::stage_queue_completion(path, job, &format!("Final {job}"), at).unwrap();
    cdr_store::schema::open_initialized(path)
        .unwrap()
        .execute(
            "INSERT INTO discord_ingress_journal
        (ingress_id,kind,channel_id,owner_user_id,payload_json,state,phase,
         owner_kind,owner_id,confirmation_delivered,created_at,updated_at)
        VALUES (?1,'message',42,1,'{}','owned','durable_prompt','prompt',?1,?2,?3,?3)",
            params![job, acknowledged, at],
        )
        .unwrap();
}

fn hold(path: &Path) -> String {
    let logical = format!(
        "{}:{THREAD};8:turn-old;64:{};",
        THREAD.len(),
        "a".repeat(64)
    );
    let key = serde_json::to_string(&(42, "completion/commentary/v1", logical, 0)).unwrap();
    delivery_receipt::begin(path, &key, "original-content-hash").unwrap();
    key
}

#[test]
fn uncertain_progress_keeps_its_job_but_allows_acknowledged_new_turn_after_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    seed(&path, "old", 1.0, true);
    seed(&path, "new", 2.0, true);
    let key = hold(&path);
    let before = std::fs::metadata(&path).unwrap().len();
    let mut chosen = Vec::new();
    for source in [Source::Commentary, Source::Final] {
        let page =
            completion_work::page(&path, source, &mut Cursor::default(), "restarted", 2).unwrap();
        assert_eq!(
            page.entries.len(),
            1,
            "old unknown progress must not block this channel"
        );
        assert_eq!(page.entries[0].turn, "turn-new");
        assert!(
            completion_work::load(&path, &page.entries[0], "restarted", 2)
                .unwrap()
                .is_some()
        );
        chosen.push(page.entries[0].clone());
    }
    assert_eq!(delivery::list_pending(&path).unwrap().len(), 2);
    assert_eq!(commentary_outbox::pending(&path).unwrap().len(), 4);
    assert_eq!(
        delivery_receipt::begin(&path, &key, "original-content-hash").unwrap(),
        delivery_receipt::ReceiptState::Unknown
    );
    assert_eq!(std::fs::metadata(&path).unwrap().len(), before);
    // An authoritative receipt restores the older job; stale work must revalidate.
    delivery_receipt::confirm(&path, &key, "123").unwrap();
    for entry in chosen {
        assert!(
            completion_work::load(&path, &entry, "restarted", 2)
                .unwrap()
                .is_none()
        );
        let page =
            completion_work::page(&path, entry.source, &mut Cursor::default(), "restarted", 2)
                .unwrap();
        assert_eq!(page.entries[0].turn, "turn-old");
    }
}

#[test]
fn held_progress_never_releases_its_own_final_or_unacknowledged_new_turn() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    seed(&path, "old", 1.0, true);
    seed(&path, "new", 2.0, false);
    let key = hold(&path);
    for source in [Source::Commentary, Source::Final] {
        let page =
            completion_work::page(&path, source, &mut Cursor::default(), "runtime", 1).unwrap();
        assert!(page.entries.is_empty());
    }
    assert_eq!(
        delivery_receipt::begin(&path, &key, "original-content-hash").unwrap(),
        delivery_receipt::ReceiptState::Unknown
    );
    assert_eq!(delivery::list_pending(&path).unwrap().len(), 2);
    assert_eq!(commentary_outbox::pending(&path).unwrap().len(), 4);
}

#[test]
fn definitely_rejected_progress_is_eligible_without_metadata_claiming_retry() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    seed(&path, "old", 1.0, true);
    seed(&path, "new", 2.0, true);
    let key = hold(&path);
    delivery_receipt::release_rejected(&path, &key).unwrap();
    for source in [Source::Commentary, Source::Final] {
        let page =
            completion_work::page(&path, source, &mut Cursor::default(), "runtime", 1).unwrap();
        assert_eq!(page.entries[0].turn, "turn-old");
    }
    let db = cdr_store::schema::open_initialized(&path).unwrap();
    let state: (Option<String>, bool) = db
        .query_row(
            "SELECT message_id,retryable FROM codex_delivery_receipts WHERE receipt_key=?",
            [&key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, (None, true));
}
