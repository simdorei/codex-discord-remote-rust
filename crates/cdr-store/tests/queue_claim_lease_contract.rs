use cdr_store::mapping::upsert_thread;
use cdr_store::queue::{
    AppServerForkHandoffError, NewAppServerForkHandoff, NewQueueJob, QueueJobState,
    STARTING_ATTEMPT_LEASE_SECONDS, begin_app_server_fork_handoff,
    complete_app_server_fork_handoff, enqueue, list, mark_running_if_claimed,
    record_start_failure_if_claimed, try_begin_attempt,
    unresolved_app_server_fork_handoff_for_source,
};
use cdr_store::schema::open_initialized;

#[test]
fn a_fresh_claim_lease_blocks_handoff_and_the_exact_caller_can_mark_running() {
    assert!((STARTING_ATTEMPT_LEASE_SECONDS - 120.0).abs() < f64::EPSILON);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("fresh.sqlite");
    setup(&path);
    let claimed = try_begin_attempt(&path, "job", &["baseline".into()], 7)
        .unwrap()
        .unwrap();

    let handoff = begin_app_server_fork_handoff(&path, request("fresh"));
    assert!(matches!(
        handoff,
        Err(AppServerForkHandoffError::StartingAttemptLeaseActive { job_id })
            if job_id == "job"
    ));
    assert_eq!(
        unresolved_app_server_fork_handoff_for_source(&path, "source").unwrap(),
        None
    );

    let running = mark_running_if_claimed(&path, &claimed, "turn")
        .unwrap()
        .expect("the exact fresh claimant still owns the state transition");
    assert_eq!(running.state, QueueJobState::Running);
    assert_eq!(running.turn_id.as_deref(), Some("turn"));
    assert_eq!(
        record_start_failure_if_claimed(&path, &claimed, "late", true).unwrap(),
        None
    );
}

#[test]
fn known_ambiguous_handoff_fences_late_claim_writes_and_preserves_the_sentinel() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ambiguous.sqlite");
    setup(&path);
    let fresh = try_begin_attempt(&path, "job", &["baseline".into()], 7)
        .unwrap()
        .unwrap();
    let ambiguous = record_start_failure_if_claimed(
        &path,
        &fresh,
        "thread/resume timed out; outcome unknown",
        true,
    )
    .unwrap()
    .unwrap();
    begin_app_server_fork_handoff(&path, request("ambiguous")).unwrap();

    assert_eq!(
        mark_running_if_claimed(&path, &ambiguous, "late-before-finalize").unwrap(),
        None
    );
    complete_app_server_fork_handoff(&path, "ambiguous", "fork", 8).unwrap();
    let sentinel = raw_job(&path);

    assert_eq!(
        mark_running_if_claimed(&path, &ambiguous, "late-after-finalize").unwrap(),
        None
    );
    assert_eq!(
        record_start_failure_if_claimed(&path, &ambiguous, "late failure", false).unwrap(),
        None
    );
    assert_eq!(
        raw_job(&path),
        sentinel,
        "late losers must not alter one byte"
    );
    assert_eq!(list(&path).unwrap()[0].state, QueueJobState::Quarantined);
}

#[test]
fn an_expired_clean_starting_lease_can_be_fenced_for_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("expired.sqlite");
    setup(&path);
    try_begin_attempt(&path, "job", &[], 7).unwrap().unwrap();
    let connection = open_initialized(&path).unwrap();
    connection
        .execute(
            "UPDATE codex_turn_queue SET updated_at = updated_at - ? WHERE job_id = 'job'",
            [STARTING_ATTEMPT_LEASE_SECONDS + 1.0],
        )
        .unwrap();
    drop(connection);

    begin_app_server_fork_handoff(&path, request("expired"))
        .expect("expired lease may be fenced for safe recovery");
    assert!(
        unresolved_app_server_fork_handoff_for_source(&path, "source")
            .unwrap()
            .is_some()
    );
}

fn setup(path: &std::path::Path) {
    upsert_thread(path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    enqueue(
        path,
        NewQueueJob {
            job_id: "job",
            target_thread_id: "source",
            channel_id: 101,
            owner_user_id: Some(7),
            discord_message_id: Some(10),
            app_server_generation: 7,
            prompt: "keep",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
}

fn request(handoff_id: &str) -> NewAppServerForkHandoff<'_> {
    NewAppServerForkHandoff {
        handoff_id,
        ambiguous_job_id: Some("job"),
        source_thread_id: "source",
        expected_generation: 7,
        quarantine_reason: "recover ambiguous start",
    }
}

fn raw_job(path: &std::path::Path) -> (String, Option<String>, String, Vec<u8>, i64, u64) {
    open_initialized(path)
        .unwrap()
        .query_row(
            "SELECT state, turn_id, last_error, CAST(baseline_turn_ids AS BLOB), \
             attempt_count, updated_at FROM codex_turn_queue WHERE job_id = 'job'",
            [],
            |row| {
                let updated_at: f64 = row.get(5)?;
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    updated_at.to_bits(),
                ))
            },
        )
        .unwrap()
}
