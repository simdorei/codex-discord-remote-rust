use cdr_store::mapping::{thread_channels, upsert_thread};
use cdr_store::queue::{
    AppServerForkHandoffError, NewAppServerForkHandoff, NewQueueJob, QueueJobState,
    begin_app_server_fork_handoff, begin_attempt,
    cancel_app_server_fork_handoff_after_definite_failure, complete_app_server_fork_handoff,
    enqueue, list, record_start_failure, unresolved_app_server_fork_handoff_for_source,
};
use cdr_store::schema::open_initialized;

#[test]
fn begin_is_fenced_by_exact_starting_job_and_existing_unique_mapping() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let path = temp.path().join("begin-fences.sqlite");
    enqueue(&path, job("ambiguous", "source", 5, 301, 1.0)).expect("enqueue job");
    begin_attempt(&path, "ambiguous", &[], 5).expect("begin attempt");
    record_start_failure(&path, "ambiguous", 5, "known ambiguous start", true)
        .expect("establish ambiguity after the fresh claim lease");

    let unmapped = begin_app_server_fork_handoff(&path, request("intent-a", 5))
        .expect("an unmapped source uses the zero snapshot sentinel");
    assert_eq!(
        (
            unmapped.handoff.discord_channel_id,
            unmapped.handoff.discord_thread_id
        ),
        (0, 0)
    );
    assert!(cancel_app_server_fork_handoff_after_definite_failure(&path, "intent-a").unwrap());
    assert_eq!(
        unresolved_app_server_fork_handoff_for_source(&path, "source").expect("read absent intent"),
        None
    );

    upsert_thread(&path, "source", "project", "Original", 900, 901, 1.0).expect("map source");
    upsert_thread(&path, "duplicate", "project", "Duplicate", 902, 901, 2.0)
        .expect("create duplicate Discord mapping fixture");
    let duplicate_mapping = begin_app_server_fork_handoff(&path, request("intent-a", 5));
    assert!(matches!(
        duplicate_mapping,
        Err(AppServerForkHandoffError::MissingOrStaleMapping { .. })
    ));

    let connection = open_initialized(&path).expect("remove duplicate fixture");
    connection
        .execute(
            "DELETE FROM mirror_threads WHERE codex_thread_id = 'duplicate'",
            [],
        )
        .expect("remove duplicate mapping");
    drop(connection);
    let begun =
        begin_app_server_fork_handoff(&path, request("intent-a", 5)).expect("begin valid intent");
    assert!(begun.created);

    let competing = begin_app_server_fork_handoff(&path, request("intent-b", 5));
    assert!(matches!(
        competing,
        Err(AppServerForkHandoffError::ConflictingIntent { .. })
    ));
    let wrong_generation = begin_app_server_fork_handoff(
        &path,
        NewAppServerForkHandoff {
            handoff_id: "intent-a",
            expected_generation: 6,
            ..request("intent-a", 6)
        },
    );
    assert!(matches!(
        wrong_generation,
        Err(AppServerForkHandoffError::ConflictingIntent { .. })
    ));
}

#[test]
fn stale_queue_cas_rolls_back_mapping_retarget_and_completion_marker() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let path = temp.path().join("stale-job.sqlite");
    setup_handoff(&path, "intent-job");
    let connection = open_initialized(&path).expect("mutate observed generation");
    connection
        .execute(
            "UPDATE codex_turn_queue SET app_server_generation = 6 WHERE job_id = 'ambiguous'",
            [],
        )
        .expect("make queue observation stale");
    drop(connection);

    let result = complete_app_server_fork_handoff(&path, "intent-job", "fork", 77);
    assert!(matches!(
        result,
        Err(AppServerForkHandoffError::StaleStartingJob { .. })
    ));
    assert_untouched_after_failed_completion(&path, 6, 901);
}

#[test]
fn stale_mapping_rolls_back_queue_and_keeps_intent_unresolved() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let path = temp.path().join("stale-mapping.sqlite");
    setup_handoff(&path, "intent-map");
    let connection = open_initialized(&path).expect("mutate observed mapping");
    connection
        .execute(
            "UPDATE mirror_threads SET discord_thread_id = 999 WHERE codex_thread_id = 'source'",
            [],
        )
        .expect("make mapping observation stale");
    drop(connection);

    let result = complete_app_server_fork_handoff(&path, "intent-map", "fork", 77);
    assert!(matches!(
        result,
        Err(AppServerForkHandoffError::MissingOrStaleMapping { .. })
    ));
    assert_untouched_after_failed_completion(&path, 5, 999);
}

#[test]
fn target_collision_and_additional_inflight_job_fail_closed() {
    for collision in ["mapped-target", "target-queue", "other-inflight"] {
        let temp = tempfile::tempdir().expect("create temp directory");
        let path = temp.path().join(format!("{collision}.sqlite"));
        if collision == "other-inflight" {
            setup_handoff_with_extra_pending(&path, collision);
        } else {
            setup_handoff(&path, collision);
        }
        match collision {
            "mapped-target" => upsert_thread(&path, "fork", "project", "Existing", 910, 911, 2.0)
                .expect("map target collision"),
            "target-queue" => {
                enqueue(&path, job("target-job", "fork", 3, 303, 3.0))
                    .expect("enqueue target collision");
            }
            "other-inflight" => {
                begin_attempt(&path, "other-start", &[], 5).expect("start second job");
            }
            _ => unreachable!(),
        }

        assert!(complete_app_server_fork_handoff(&path, collision, "fork", 77).is_err());
        assert_eq!(
            find(&list(&path).unwrap(), "ambiguous").state,
            QueueJobState::Starting
        );
        assert_eq!(
            find(&list(&path).unwrap(), "pending").target_thread_id,
            "source"
        );
        assert_eq!(thread_channels(&path, "source").unwrap(), Some((900, 901)));
        assert!(
            unresolved_app_server_fork_handoff_for_source(&path, "source")
                .unwrap()
                .is_some()
        );
    }
}

fn setup_handoff(path: &std::path::Path, handoff_id: &str) {
    setup_handoff_inner(path, handoff_id, false);
}

fn setup_handoff_with_extra_pending(path: &std::path::Path, handoff_id: &str) {
    setup_handoff_inner(path, handoff_id, true);
}

fn setup_handoff_inner(path: &std::path::Path, handoff_id: &str, extra_pending: bool) {
    upsert_thread(path, "source", "project", "Original", 900, 901, 1.0).expect("map source");
    enqueue(path, job("ambiguous", "source", 5, 301, 1.0)).expect("enqueue ambiguous");
    begin_attempt(path, "ambiguous", &["baseline".into()], 5).expect("begin attempt");
    record_start_failure(path, "ambiguous", 5, "known ambiguous start", true)
        .expect("establish ambiguity after the fresh claim lease");
    enqueue(path, job("pending", "source", 5, 302, 2.0)).expect("enqueue pending");
    if extra_pending {
        enqueue(path, job("other-start", "source", 5, 304, 3.0))
            .expect("enqueue legacy writer fixture before the handoff fence");
    }
    begin_app_server_fork_handoff(path, request(handoff_id, 5)).expect("begin handoff");
}

fn assert_untouched_after_failed_completion(
    path: &std::path::Path,
    ambiguous_generation: i64,
    discord_thread_id: i64,
) {
    let jobs = list(path).expect("list queue after rollback");
    let ambiguous = find(&jobs, "ambiguous");
    assert_eq!(ambiguous.state, QueueJobState::Starting);
    assert_eq!(ambiguous.app_server_generation, ambiguous_generation);
    let pending = find(&jobs, "pending");
    assert_eq!(pending.state, QueueJobState::Pending);
    assert_eq!(pending.target_thread_id, "source");
    assert_eq!(pending.app_server_generation, 5);
    assert_eq!(
        thread_channels(path, "source").unwrap(),
        Some((900, discord_thread_id))
    );
    assert_eq!(thread_channels(path, "fork").unwrap(), None);
    assert!(
        unresolved_app_server_fork_handoff_for_source(path, "source")
            .unwrap()
            .is_some()
    );
}

fn request(handoff_id: &str, generation: i64) -> NewAppServerForkHandoff<'_> {
    NewAppServerForkHandoff {
        handoff_id,
        ambiguous_job_id: Some("ambiguous"),
        source_thread_id: "source",
        expected_generation: generation,
        quarantine_reason: "ambiguous start",
    }
}

fn job<'a>(
    id: &'a str,
    target: &'a str,
    generation: i64,
    message: i64,
    created_at: f64,
) -> NewQueueJob<'a> {
    NewQueueJob {
        job_id: id,
        target_thread_id: target,
        channel_id: 901,
        owner_user_id: Some(88),
        discord_message_id: Some(message),
        app_server_generation: generation,
        prompt: id,
        queued: true,
        ack_sent: true,
        created_at,
    }
}

fn find<'a>(
    jobs: &'a [cdr_store::queue::StoredQueueJob],
    id: &str,
) -> &'a cdr_store::queue::StoredQueueJob {
    jobs.iter()
        .find(|job| job.job_id == id)
        .expect("job exists")
}
