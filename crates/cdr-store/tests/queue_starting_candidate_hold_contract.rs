use std::path::Path;
use std::sync::{Arc, Barrier};

use cdr_store::delivery::{complete as complete_delivery, list_pending};
use cdr_store::queue::{
    NewQueueJob, QueueJobState, STARTING_CANDIDATE_HOLD_PREFIX, begin_attempt, enqueue,
    hold_starting_for_ambiguous_candidates_if_claimed, list, record_start_failure,
};

const NOTICE_ID: &str = "turn-start-candidates-ambiguous:held";

#[test]
fn hold_is_sticky_visible_latest_and_legacy_compatible_without_notice_recreation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("hold.sqlite");
    let claimed = seed_starting(&path, "original transport timeout");
    enqueue(&path, job("following", "wait behind hold", 2.0)).unwrap();

    let held = hold_starting_for_ambiguous_candidates_if_claimed(
        &path,
        &claimed,
        &["turn-z".into(), "turn-a".into(), "turn-z".into()],
    )
    .unwrap()
    .expect("the exact Starting snapshot wins its hold CAS");

    assert_eq!(held.state, QueueJobState::Starting);
    assert_eq!(held.turn_id, None);
    assert!(held.last_error.starts_with(STARTING_CANDIDATE_HOLD_PREFIX));
    assert!(held.last_error.contains("candidate_count=2"));
    assert!(held.last_error.find("turn-a") < held.last_error.find("turn-z"));
    assert!(held.last_error.contains("original transport timeout"));
    let jobs = list(&path).unwrap();
    assert_eq!(jobs[1].state, QueueJobState::Pending);
    let first = list_pending(&path).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].delivery_id, NOTICE_ID);
    assert_eq!(first[0].job_id, NOTICE_ID);
    assert_ne!(first[0].job_id, held.job_id);
    assert!(first[0].content.contains("held"));
    assert!(first[0].content.contains("source"));

    let refreshed = hold_starting_for_ambiguous_candidates_if_claimed(
        &path,
        &held,
        &["turn-d".into(), "turn-c".into()],
    )
    .unwrap()
    .expect("the marked exact snapshot remains held");
    assert_eq!(
        refreshed, held,
        "notice refresh must not rewrite the queue row"
    );
    let latest = list_pending(&path).unwrap();
    assert_eq!(latest.len(), 1);
    assert!(latest[0].content.contains("turn-c"));
    assert!(latest[0].content.contains("turn-d"));
    assert!(!latest[0].content.contains("turn-a"));

    assert!(complete_delivery(&path, NOTICE_ID).unwrap());
    assert!(
        hold_starting_for_ambiguous_candidates_if_claimed(&path, &held, &[])
            .unwrap()
            .is_some()
    );
    assert!(list_pending(&path).unwrap().is_empty());
    assert_legacy_starting_hold_storage_contract(&path);
}

#[test]
fn two_exact_snapshot_writers_have_one_cas_winner_and_one_notice() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("race.sqlite");
    let claimed = seed_starting(&path, "");
    let barrier = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let path = path.clone();
            let claimed = claimed.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                hold_starting_for_ambiguous_candidates_if_claimed(
                    &path,
                    &claimed,
                    &["turn-b".into(), "turn-a".into()],
                )
                .unwrap()
            })
        })
        .collect::<Vec<_>>();

    let winners = handles
        .into_iter()
        .map(|handle| handle.join().unwrap().is_some())
        .filter(|won| *won)
        .count();

    assert_eq!(winners, 1);
    let jobs = list(&path).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].state, QueueJobState::Starting);
    assert!(
        jobs[0]
            .last_error
            .starts_with(STARTING_CANDIDATE_HOLD_PREFIX)
    );
    assert_eq!(list_pending(&path).unwrap().len(), 1);
}

#[test]
fn candidate_marker_and_discord_notice_are_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("bounded.sqlite");
    let claimed = seed_starting(&path, &"previous".repeat(1_000));
    let candidates = (0..100)
        .map(|index| format!("candidate-{index:03}-{}", "x".repeat(500)))
        .collect::<Vec<_>>();

    let held = hold_starting_for_ambiguous_candidates_if_claimed(&path, &claimed, &candidates)
        .unwrap()
        .unwrap();

    assert!(held.last_error.chars().count() <= 1_000);
    assert!(held.last_error.contains("candidate_count=100"));
    let notices = list_pending(&path).unwrap();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].content.chars().count() <= 1_900);
    assert!(notices[0].content.contains("count=100"));
}

fn seed_starting(path: &Path, prior_error: &str) -> cdr_store::queue::StoredQueueJob {
    enqueue(path, job("held", "ambiguous work", 1.0)).unwrap();
    begin_attempt(path, "held", &["baseline".into()], 9).unwrap();
    if prior_error.is_empty() {
        list(path).unwrap().remove(0)
    } else {
        record_start_failure(path, "held", 9, prior_error, true).unwrap()
    }
}

fn job<'a>(id: &'a str, prompt: &'a str, created_at: f64) -> NewQueueJob<'a> {
    NewQueueJob {
        job_id: id,
        target_thread_id: "source",
        channel_id: 70,
        owner_user_id: Some(10),
        discord_message_id: None,
        app_server_generation: 9,
        prompt,
        queued: true,
        ack_sent: true,
        created_at,
    }
}

fn assert_legacy_starting_hold_storage_contract(path: &Path) {
    // Frozen legacy column/state contract; independent of the Rust queue decoder.
    // Executable Python rollback is intentionally retired, not required by this check.
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let rows = connection
        .prepare("SELECT state, turn_id FROM codex_turn_queue WHERE job_id=?")
        .unwrap()
        .query_map(["held"], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0], ("starting".into(), None));
}
