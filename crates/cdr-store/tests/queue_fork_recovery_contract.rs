use cdr_store::delivery::list_pending;
use cdr_store::mapping::{
    MirrorDetailMode, get_detail_mode, set_detail_mode, thread_channels, upsert_thread,
};
use cdr_store::queue::{
    AppServerForkHandoffError, NewAppServerForkHandoff, NewQueueJob, QueueJobState,
    begin_app_server_fork_handoff, begin_attempt,
    cancel_app_server_fork_handoff_after_definite_failure,
    completed_app_server_fork_target_for_source, enqueue, finalize_app_server_fork_handoff, list,
    record_app_server_fork_finalize_failure, record_start_failure, stage_app_server_fork_target,
    unresolved_app_server_fork_handoff_for_source,
};
use cdr_store::schema::open_initialized;

#[test]
fn observed_target_survives_the_crash_window_and_finalize_moves_only_owned_state() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let path = temp.path().join("observed.sqlite");
    setup_ambiguous_handoff(&path, "recover");
    seed_state_that_must_not_move(&path);

    let staged = stage_app_server_fork_target(&path, "recover", "fork")
        .expect("persist successful fork response before mapping finalize");
    assert_eq!(staged.observed_target_thread_id.as_deref(), Some("fork"));
    assert_eq!(staged.target_thread_id, None);
    assert_eq!(thread_channels(&path, "source").unwrap(), Some((100, 101)));
    assert_eq!(
        stage_app_server_fork_target(&path, "recover", "fork").unwrap(),
        staged
    );

    drop(open_initialized(&path).expect("simulate process restart"));
    let recovered = unresolved_app_server_fork_handoff_for_source(&path, "source")
        .unwrap()
        .expect("unresolved intent survives restart");
    assert_eq!(recovered.observed_target_thread_id.as_deref(), Some("fork"));

    let completed = finalize_app_server_fork_handoff(&path, "recover", 9)
        .expect("restart finalizes the already observed target");
    assert!(completed.applied);
    assert_eq!(thread_channels(&path, "source").unwrap(), None);
    assert_eq!(thread_channels(&path, "fork").unwrap(), Some((100, 101)));
    assert_eq!(
        get_detail_mode(&path, "fork").unwrap(),
        MirrorDetailMode::All
    );
    assert_eq!(
        get_detail_mode(&path, "source").unwrap(),
        MirrorDetailMode::Send
    );
    assert_eq!(
        completed_app_server_fork_target_for_source(&path, "source").unwrap(),
        Some("fork".into())
    );
    assert_unowned_state_stayed_on_source(&path);

    let jobs = list(&path).unwrap();
    let quarantined = jobs.iter().find(|job| job.job_id == "ambiguous").unwrap();
    assert_eq!(quarantined.state, QueueJobState::Quarantined);
    assert!(quarantined.last_error.contains("fork ownership recovery"));
    assert!(quarantined.last_error.contains("original timeout detail"));

    let deliveries = list_pending(&path).expect("terminal quarantine delivery is durable");
    assert_eq!(deliveries.len(), 2);
    let delivery = deliveries
        .iter()
        .find(|delivery| delivery.delivery_id == "quarantine:ambiguous")
        .unwrap();
    assert_eq!(delivery.delivery_id, "quarantine:ambiguous");
    assert_eq!(delivery.job_id, "ambiguous");
    assert_eq!(delivery.channel_id, 99);
    assert_eq!(delivery.turn_id, quarantined.turn_id.clone().unwrap());
    assert!(delivery.content.contains("not retried"));
    assert!(delivery.content.contains("fork ownership recovery"));
    assert!(delivery.content.contains("original timeout detail"));

    let repeated =
        finalize_app_server_fork_handoff(&path, "recover", 100).expect("re-finalize is idempotent");
    assert!(!repeated.applied);
    assert_eq!(list_pending(&path).unwrap().len(), 2);
}

#[test]
fn observed_success_cannot_be_cancelled_as_a_definite_failure() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let path = temp.path().join("cancel-observed.sqlite");
    setup_proactive_handoff(&path, "observed");
    assert!(matches!(
        finalize_app_server_fork_handoff(&path, "observed", 9),
        Err(AppServerForkHandoffError::ForkTargetNotObserved { .. })
    ));
    stage_app_server_fork_target(&path, "observed", "fork").unwrap();

    let cancellation = cancel_app_server_fork_handoff_after_definite_failure(&path, "observed");
    assert!(matches!(
        cancellation,
        Err(AppServerForkHandoffError::ForkTargetAlreadyObserved { .. })
    ));
    assert_eq!(
        unresolved_app_server_fork_handoff_for_source(&path, "source")
            .unwrap()
            .unwrap()
            .observed_target_thread_id
            .as_deref(),
        Some("fork")
    );
}

#[test]
fn target_collision_is_recorded_before_finalize_rejects_it() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let path = temp.path().join("observed-collision.sqlite");
    upsert_thread(&path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    enqueue(&path, job("pending", "source", 4, 12)).unwrap();
    setup_proactive_intent_only(&path, "collision");
    upsert_thread(&path, "fork", "project", "Existing", 300, 301, 2.0).unwrap();

    let staged = stage_app_server_fork_target(&path, "collision", "fork")
        .expect("the RPC response is an observation even when local state collides");
    assert_eq!(staged.observed_target_thread_id.as_deref(), Some("fork"));
    let finalize_error = finalize_app_server_fork_handoff(&path, "collision", 9).unwrap_err();
    assert!(matches!(
        &finalize_error,
        AppServerForkHandoffError::TargetConflict { .. }
    ));
    record_app_server_fork_finalize_failure(&path, "collision", &finalize_error.to_string())
        .expect("observed target and exact finalize failure remain durable");
    let unresolved = unresolved_app_server_fork_handoff_for_source(&path, "source")
        .unwrap()
        .unwrap();
    assert_eq!(
        unresolved.observed_target_thread_id.as_deref(),
        Some("fork")
    );
    assert!(unresolved.last_fork_error.contains("already in use"));
    assert_eq!(thread_channels(&path, "source").unwrap(), Some((100, 101)));
    assert!(
        list(&path).unwrap()[0]
            .last_error
            .starts_with(cdr_store::queue::UNRESOLVED_FORK_ERROR_PREFIX)
    );
    assert_eq!(list_pending(&path).unwrap().len(), 1);

    drop(open_initialized(&path).expect("simulate restart after finalize failure"));
    open_initialized(&path)
        .unwrap()
        .execute(
            "DELETE FROM mirror_threads WHERE codex_thread_id = 'fork'",
            [],
        )
        .unwrap();
    finalize_app_server_fork_handoff(&path, "collision", 9)
        .expect("restart finalizes the already observed target after collision clears");
    assert!(list_pending(&path).unwrap().is_empty());
    let pending = list(&path).unwrap().pop().unwrap();
    assert_eq!(pending.target_thread_id, "fork");
    assert!(pending.last_error.is_empty());
}

#[test]
fn mapping_drift_finalize_failure_is_durable_until_restart_can_repair_it() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let path = temp.path().join("mapping-drift.sqlite");
    upsert_thread(&path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    enqueue(&path, job("pending", "source", 4, 13)).unwrap();
    setup_proactive_intent_only(&path, "mapping-drift");
    stage_app_server_fork_target(&path, "mapping-drift", "fork").unwrap();
    upsert_thread(&path, "source", "project", "Moved", 200, 201, 2.0).unwrap();

    let finalize_error = finalize_app_server_fork_handoff(&path, "mapping-drift", 9).unwrap_err();
    assert!(matches!(
        &finalize_error,
        AppServerForkHandoffError::MissingOrStaleMapping { .. }
    ));
    let recorded = record_app_server_fork_finalize_failure(
        &path,
        "mapping-drift",
        &finalize_error.to_string(),
    )
    .unwrap();
    assert_eq!(recorded.observed_target_thread_id.as_deref(), Some("fork"));
    assert!(
        recorded
            .last_fork_error
            .contains("missing, stale, or duplicated")
    );
    assert_eq!(list_pending(&path).unwrap().len(), 1);

    drop(open_initialized(&path).expect("simulate restart"));
    let recovered = unresolved_app_server_fork_handoff_for_source(&path, "source")
        .unwrap()
        .unwrap();
    assert_eq!(recovered.observed_target_thread_id.as_deref(), Some("fork"));
    assert_eq!(recovered.last_fork_error, recorded.last_fork_error);

    upsert_thread(&path, "source", "project", "Source", 100, 101, 3.0).unwrap();
    finalize_app_server_fork_handoff(&path, "mapping-drift", 9).unwrap();
    assert_eq!(thread_channels(&path, "fork").unwrap(), Some((100, 101)));
    assert!(list_pending(&path).unwrap().is_empty());
}

#[test]
fn old_handoff_table_is_upgraded_additively_before_staging() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let path = temp.path().join("legacy-handoff.sqlite");
    upsert_thread(&path, "source", "project", "Source", 200, 201, 1.0).unwrap();
    let connection = open_initialized(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE codex_thread_fork_handoffs (\
              handoff_id TEXT PRIMARY KEY, ambiguous_job_id TEXT UNIQUE, \
              source_thread_id TEXT NOT NULL UNIQUE, expected_generation INTEGER NOT NULL, \
              discord_channel_id INTEGER NOT NULL, discord_thread_id INTEGER NOT NULL, \
              quarantine_reason TEXT NOT NULL, target_thread_id TEXT UNIQUE, \
              completed_generation INTEGER, created_at REAL NOT NULL, completed_at REAL, \
              CHECK ((target_thread_id IS NULL AND completed_generation IS NULL AND completed_at IS NULL) \
                  OR (target_thread_id IS NOT NULL AND completed_generation IS NOT NULL AND completed_at IS NOT NULL))\
            );",
        )
        .unwrap();
    drop(connection);

    setup_proactive_intent_only(&path, "legacy");
    let staged = stage_app_server_fork_target(&path, "legacy", "fork").unwrap();
    assert_eq!(staged.observed_target_thread_id.as_deref(), Some("fork"));
    let connection = open_initialized(&path).unwrap();
    let columns = connection
        .prepare("PRAGMA table_info(codex_thread_fork_handoffs)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(
        columns
            .iter()
            .any(|column| column == "observed_target_thread_id")
    );
    assert!(columns.iter().any(|column| column == "last_fork_error"));
    assert!(
        columns
            .iter()
            .any(|column| column == "fork_failure_ambiguous")
    );
}

fn setup_ambiguous_handoff(path: &std::path::Path, handoff_id: &str) {
    upsert_thread(path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    set_detail_mode(path, "source", MirrorDetailMode::All).unwrap();
    enqueue(path, job("ambiguous", "source", 4, 10)).unwrap();
    begin_attempt(path, "ambiguous", &["before".into()], 4).unwrap();
    record_start_failure(path, "ambiguous", 4, "original timeout detail", true).unwrap();
    enqueue(path, job("pending", "source", 4, 11)).unwrap();
    begin_app_server_fork_handoff(
        path,
        NewAppServerForkHandoff {
            handoff_id,
            ambiguous_job_id: Some("ambiguous"),
            source_thread_id: "source",
            expected_generation: 4,
            quarantine_reason: "fork ownership recovery",
        },
    )
    .unwrap();
}

fn setup_proactive_handoff(path: &std::path::Path, handoff_id: &str) {
    upsert_thread(path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    setup_proactive_intent_only(path, handoff_id);
}

fn setup_proactive_intent_only(path: &std::path::Path, handoff_id: &str) {
    begin_app_server_fork_handoff(
        path,
        NewAppServerForkHandoff {
            handoff_id,
            ambiguous_job_id: None,
            source_thread_id: "source",
            expected_generation: 4,
            quarantine_reason: "ownership",
        },
    )
    .unwrap();
}

fn seed_state_that_must_not_move(path: &std::path::Path) {
    let connection = open_initialized(path).unwrap();
    connection
        .execute_batch(
            "INSERT INTO codex_session_mirror_offsets \
                 (codex_thread_id, rollout_path, cursor, updated_at) \
                 VALUES ('source', 'rollout.jsonl', 12, 1.0); \
             INSERT INTO codex_session_mirror_events \
                 (event_digest, codex_thread_id, created_at) \
                 VALUES ('event', 'source', 1.0); \
             INSERT INTO busy_choices \
                 (choice_id, owner_user_id, channel_id, target_thread_id, prompt, \
                  allow_steer, created_at, expires_at, claimed_at) \
                 VALUES ('busy', 1, 2, 'source', 'keep', 0, 1.0, 2.0, NULL); \
             INSERT INTO codex_delivery_outbox \
                 (delivery_id, job_id, target_thread_id, turn_id, channel_id, content, \
                  created_at, updated_at) \
                 VALUES ('old-delivery', 'old-delivery-job', 'source', 'old-turn', 99, \
                         'keep', 1.0, 1.0);",
        )
        .unwrap();
}

fn assert_unowned_state_stayed_on_source(path: &std::path::Path) {
    let connection = open_initialized(path).unwrap();
    for (table, column) in [
        ("codex_session_mirror_offsets", "codex_thread_id"),
        ("codex_session_mirror_events", "codex_thread_id"),
        ("busy_choices", "target_thread_id"),
    ] {
        let count: i64 = connection
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {column} = 'source'"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "{table} must not be retargeted");
    }
    let old_delivery_on_source: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM codex_delivery_outbox \
             WHERE delivery_id = 'old-delivery' AND target_thread_id = 'source')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(old_delivery_on_source, "existing outbox rows must not move");
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
        created_at: f64::from(i32::try_from(message_id).expect("fixture message id fits i32")),
    }
}
