use std::sync::{Arc, Barrier};

use cdr_store::mapping::upsert_thread;
use cdr_store::queue::{
    NewAppServerForkHandoff, NewQueueJob, QueueJobState, begin_app_server_fork_handoff,
    cancel_app_server_fork_handoff_after_definite_failure, enqueue, list, try_begin_attempt,
};
use cdr_store::schema::open_initialized;

#[test]
fn try_begin_attempt_is_a_single_pending_cas_and_initializes_its_fence_table() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let path = temp.path().join("claim.sqlite");
    enqueue(&path, job("job", "free", 7, 1)).expect("enqueue pending job");

    let barrier = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                try_begin_attempt(&path, "job", &["baseline".into()], 7)
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("claim thread does not panic").unwrap())
        .collect::<Vec<_>>();

    assert_eq!(results.iter().filter(|result| result.is_some()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_none()).count(), 1);
    let stored = list(&path).unwrap().pop().expect("job remains durable");
    assert_eq!(stored.state, QueueJobState::Starting);
    assert_eq!(stored.attempt_count, 1);
    assert_eq!(stored.baseline_turn_ids, vec!["baseline"]);

    let connection = open_initialized(&path).expect("inspect initialized fence table");
    let table_exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' \
             AND name = 'codex_thread_fork_handoffs')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(table_exists);
}

#[test]
fn unresolved_handoff_fences_claim_until_a_definite_failure_is_cancelled() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let path = temp.path().join("fence.sqlite");
    upsert_thread(&path, "source", "project", "Source", 10, 11, 1.0).unwrap();
    enqueue(&path, job("pending", "source", 4, 2)).unwrap();
    begin_app_server_fork_handoff(
        &path,
        NewAppServerForkHandoff {
            handoff_id: "intent",
            ambiguous_job_id: None,
            source_thread_id: "source",
            expected_generation: 4,
            quarantine_reason: "ownership fork",
        },
    )
    .expect("persist fork fence");

    assert_eq!(try_begin_attempt(&path, "pending", &[], 4).unwrap(), None);
    let still_pending = list(&path).unwrap().pop().unwrap();
    assert_eq!(still_pending.state, QueueJobState::Pending);
    assert_eq!(still_pending.attempt_count, 0);

    assert!(cancel_app_server_fork_handoff_after_definite_failure(&path, "intent").unwrap());
    assert!(!cancel_app_server_fork_handoff_after_definite_failure(&path, "intent").unwrap());
    let claimed = try_begin_attempt(&path, "pending", &[], 4)
        .unwrap()
        .expect("definite cancellation releases claim fence");
    assert_eq!(claimed.state, QueueJobState::Starting);
    assert_eq!(claimed.attempt_count, 1);
}

#[test]
fn an_ambiguous_unresolved_fork_stays_fenced_across_reopen() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let path = temp.path().join("ambiguous.sqlite");
    upsert_thread(&path, "source", "project", "Source", 20, 21, 1.0).unwrap();
    enqueue(&path, job("pending", "source", 5, 3)).unwrap();
    begin_app_server_fork_handoff(
        &path,
        NewAppServerForkHandoff {
            handoff_id: "ambiguous-intent",
            ambiguous_job_id: None,
            source_thread_id: "source",
            expected_generation: 5,
            quarantine_reason: "transport outcome unknown",
        },
    )
    .unwrap();

    drop(open_initialized(&path).expect("simulate reopen"));
    assert_eq!(try_begin_attempt(&path, "pending", &[], 5).unwrap(), None);
    assert_eq!(list(&path).unwrap()[0].state, QueueJobState::Pending);
}

fn job<'a>(id: &'a str, target: &'a str, generation: i64, message_id: i64) -> NewQueueJob<'a> {
    NewQueueJob {
        job_id: id,
        target_thread_id: target,
        channel_id: 99,
        owner_user_id: Some(7),
        discord_message_id: Some(message_id),
        app_server_generation: generation,
        prompt: "keep",
        queued: true,
        ack_sent: true,
        created_at: 1.0,
    }
}
