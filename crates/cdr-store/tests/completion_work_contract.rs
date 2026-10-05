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
