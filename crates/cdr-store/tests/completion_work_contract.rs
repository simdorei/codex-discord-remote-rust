use cdr_store::{
    completion_work::{self, Cursor, Source},
    delivery, queue,
};
use std::path::Path;

fn seed(path: &Path, job: &str, channel: i64, content: &str, now: f64) {
    queue::enqueue(
        path,
        queue::NewQueueJob {
            job_id: job,
            target_thread_id: job,
            channel_id: channel,
            owner_user_id: Some(1),
            discord_message_id: None,
            app_server_generation: 1,
            prompt: "fixture",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    queue::begin_attempt(path, job, &[], 1).unwrap();
    queue::mark_running(path, job, "turn", 1).unwrap();
    delivery::stage_queue_completion(path, job, content, now).unwrap();
}

#[test]
fn finite_keyset_pages_cross_128_old_heads_without_loading_their_bodies() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    for n in 0..140 {
        seed(
            &path,
            &format!("job-{n:03}"),
            100 + n,
            "large fixture body",
            1.0,
        );
    }
    let mut cursor = Cursor::default();
    let first = completion_work::page(&path, Source::Final, &mut cursor, "runtime", 1).unwrap();
    assert_eq!(first.entries.len(), completion_work::PAGE_SIZE);
    seed(&path, "zz-later", 999, "later", 2.0);
    let mut ids = first.entries.into_iter().map(|e| e.id).collect::<Vec<_>>();
    while !cursor.finished {
        let page = completion_work::page(&path, Source::Final, &mut cursor, "runtime", 1).unwrap();
        assert!(page.entries.len() <= completion_work::PAGE_SIZE);
        ids.extend(page.entries.into_iter().map(|e| e.id));
    }
    assert_eq!(ids.len(), 140);
    assert_eq!(ids.last().unwrap(), "job-139");
    assert!(
        !ids.iter().any(|id| id == "zz-later"),
        "arrivals do not extend a finite pass"
    );
}

#[test]
fn a_backlog_above_16_and_128_retains_one_head_without_hiding_b() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    for n in 0..140 {
        seed(&path, &format!("a-{n:03}"), 42, "A", 1.0);
    }
    seed(&path, "b", 43, "B", 2.0);
    let page =
        completion_work::page(&path, Source::Final, &mut Cursor::default(), "runtime", 1).unwrap();
    assert_eq!(page.entries.len(), 2);
    assert_eq!(page.entries[0].id, "a-000");
    assert_eq!(page.entries[1].id, "b");
    assert_eq!(delivery::list_pending(&path).unwrap().len(), 141);
}

#[test]
fn oversized_payload_and_identity_keep_the_original_head_and_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    seed(
        &path,
        "large",
        42,
        &"x".repeat(completion_work::MAX_PAYLOAD_BYTES + 1),
        1.0,
    );
    let page =
        completion_work::page(&path, Source::Final, &mut Cursor::default(), "runtime", 1).unwrap();
    assert!(completion_work::load(&path, &page.entries[0], "runtime", 1).is_err());
    seed(
        &path,
        &"y".repeat(completion_work::MAX_METADATA_BYTES + 1),
        43,
        "old",
        1.0,
    );
    seed(
        &path,
        "later-same-channel",
        43,
        "must remain behind old",
        2.0,
    );
    let page =
        completion_work::page(&path, Source::Final, &mut Cursor::default(), "runtime", 1).unwrap();
    assert!(page.oversized_identity);
    assert!(page.entries.iter().all(|e| e.channel != 43));
    assert_eq!(delivery::list_pending(&path).unwrap().len(), 3);
}

#[test]
fn held_receipt_does_not_promote_a_later_same_channel_head_or_claim_a_retry() {
    use sha2::Digest as _;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    seed(&path, "old", 42, "old", 1.0);
    seed(&path, "later", 42, "later", 2.0);
    let key = serde_json::to_string(&(42, "completion/v1", "old", 0)).unwrap();
    cdr_store::delivery_receipt::begin(&path, &key, &hex::encode(sha2::Sha256::digest(b"old")))
        .unwrap();
    let held =
        completion_work::page(&path, Source::Final, &mut Cursor::default(), "runtime", 1).unwrap();
    assert!(held.entries.is_empty());
    assert_eq!(held.held_receipt_heads, 1);
    assert!(
        completion_work::heads_for_target(&path, "later", "runtime", 1)
            .unwrap()
            .is_empty()
    );
    assert_eq!(delivery::list_pending(&path).unwrap().len(), 2);
    // Simulate an authoritative definite rejection, not a timed-out POST.
    cdr_store::delivery_receipt::release_rejected(&path, &key).unwrap();
    let allowed =
        completion_work::page(&path, Source::Final, &mut Cursor::default(), "runtime", 1).unwrap();
    assert_eq!(allowed.entries[0].id, "old");
    let retryable: bool = cdr_store::schema::open_initialized(&path)
        .unwrap()
        .query_row(
            "SELECT retryable FROM codex_delivery_receipts WHERE receipt_key=?",
            [&key],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        retryable,
        "metadata discovery must not acquire a receipt claim"
    );
    cdr_store::delivery_receipt::confirm(&path, &key, "123").unwrap();
    assert!(
        completion_work::load(&path, &allowed.entries[0], "runtime", 1)
            .unwrap()
            .is_some()
    );
}

fn first_reply(path: &Path, job: &str, suffix: &str, confirmed: bool, created: f64) {
    cdr_store::schema::open_initialized(path)
        .unwrap()
        .execute(
            "INSERT INTO discord_ingress_journal
         (ingress_id,kind,channel_id,owner_user_id,payload_json,state,phase,
          owner_kind,owner_id,confirmation_delivered,created_at,updated_at)
         VALUES (?1,'message',42,1,'{}','owned','durable_prompt','prompt',?2,?3,?4,?4)",
            rusqlite::params![format!("message:{job}:{suffix}"), job, confirmed, created],
        )
        .unwrap();
}

#[test]
fn unconfirmed_first_reply_retains_old_output_without_blocking_new_job() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    for (job, confirmed, stamp) in [("old", false, 1.0), ("new", true, 2.0)] {
        seed(&path, job, 42, job, stamp);
        first_reply(&path, job, "original", confirmed, stamp);
        let db = cdr_store::schema::open_initialized(&path).unwrap();
        db.execute("UPDATE codex_delivery_outbox SET target_thread_id='same-thread',turn_id=? WHERE job_id=?",
            rusqlite::params![format!("turn-{job}"), job]).unwrap();
        db.execute(
            "INSERT INTO codex_commentary_outbox
            (delivery_key,job_id,target_thread_id,turn_id,channel_id,text)
            VALUES (?1,?1,'same-thread',?2,42,?1)",
            rusqlite::params![job, format!("turn-{job}")],
        )
        .unwrap();
    }
    let mut chosen = Vec::new();
    for source in [Source::Commentary, Source::Final] {
        let page =
            completion_work::page(&path, source, &mut Cursor::default(), "runtime", 1).unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_eq!(
            page.entries[0].turn, "turn-new",
            "blocked acknowledgement must not own the channel head"
        );
        assert!(
            completion_work::load(&path, &page.entries[0], "runtime", 1)
                .unwrap()
                .is_some()
        );
        chosen.push(page.entries[0].clone());
    }
    assert_eq!(delivery::list_pending(&path).unwrap().len(), 2);
    assert_eq!(
        cdr_store::commentary_outbox::pending(&path).unwrap().len(),
        2
    );
    assert!(
        cdr_store::first_reply::pending(&path, "old")
            .unwrap()
            .is_some()
    );
    // A late authoritative ACK restores the old head; stale discovery cannot overtake it.
    cdr_store::schema::open_initialized(&path)
        .unwrap()
        .execute(
            "UPDATE discord_ingress_journal SET confirmation_delivered=1 WHERE owner_id='old'",
            [],
        )
        .unwrap();
    for entry in chosen {
        assert!(
            completion_work::load(&path, &entry, "runtime", 1)
                .unwrap()
                .is_none()
        );
        let page = completion_work::page(&path, entry.source, &mut Cursor::default(), "runtime", 1)
            .unwrap();
        assert_eq!(page.entries[0].turn, "turn-old");
    }
}

#[test]
fn duplicate_unconfirmed_ingress_does_not_hide_an_acknowledged_original() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    seed(&path, "original", 42, "answer", 1.0);
    first_reply(&path, "original", "first", true, 1.0);
    first_reply(&path, "original", "duplicate", false, 2.0);
    assert!(
        cdr_store::first_reply::pending(&path, "original")
            .unwrap()
            .is_none()
    );
    let page =
        completion_work::page(&path, Source::Final, &mut Cursor::default(), "runtime", 1).unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].id, "original");
}
