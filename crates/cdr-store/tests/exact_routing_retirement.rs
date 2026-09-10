use cdr_store::{
    mapping::{mirrored_thread_id, upsert_thread},
    queue::{
        NewAppServerForkHandoff, begin_app_server_fork_handoff, stage_app_server_fork_target,
        unresolved_app_server_fork_handoff_for_source,
    },
};

#[test]
#[ignore = "explicit read-only snapshot of an operator-selected store"]
fn live_snapshot_retirement_preserves_all_rooms_and_saved_requests() {
    let source =
        std::env::var_os("CDR_ROUTING_STORE_COPY_SOURCE").expect("explicit source required");
    let source =
        rusqlite::Connection::open_with_flags(source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("snapshot.sqlite");
    let mut copy = rusqlite::Connection::open(&db).unwrap();
    rusqlite::backup::Backup::new(&source, &mut copy)
        .unwrap()
        .run_to_completion(128, std::time::Duration::from_millis(5), None)
        .unwrap();
    drop(copy);
    let rooms = cdr_store::mapping::mirror_targets(&db, i64::MAX).unwrap();
    let requests = cdr_store::prompt_intake::list_prompt_intakes(&db).unwrap();
    let jobs = cdr_store::queue::list(&db).unwrap();
    let retired = cdr_store::queue::retire_copy_only_handoffs(&db).unwrap();
    if let Ok(expected) = std::env::var("CDR_ROUTING_EXPECT_RETIRED_SOURCE") {
        assert!(
            unresolved_app_server_fork_handoff_for_source(&db, &expected)
                .unwrap()
                .is_none(),
            "selected copy-only blocker must be retired in the snapshot"
        );
    }
    assert_eq!(
        cdr_store::mapping::mirror_targets(&db, i64::MAX).unwrap(),
        rooms
    );
    assert_eq!(
        cdr_store::prompt_intake::list_prompt_intakes(&db).unwrap(),
        requests
    );
    assert_eq!(cdr_store::queue::list(&db).unwrap(), jobs);
    let check: String = rusqlite::Connection::open(&db)
        .unwrap()
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(check, "ok");
    println!(
        "snapshot-only: retired={retired}; rooms={}, saved_requests={}, jobs={} preserved; integrity=ok",
        rooms.len(),
        requests.len(),
        jobs.len()
    );
}

#[test]
fn retires_copy_only_fence_without_moving_or_deleting_either_room() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "source", "project", "Original", 1, 2, 1.0).unwrap();
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "handoff",
            ambiguous_job_id: None,
            source_thread_id: "source",
            expected_generation: 1,
            quarantine_reason: "app-server-only ownership fork",
        },
    )
    .unwrap();
    stage_app_server_fork_target(&db, "handoff", "child").unwrap();
    upsert_thread(&db, "child", "project", "Copy", 1, 3, 1.0).unwrap();
    assert_eq!(cdr_store::queue::retire_copy_only_handoffs(&db).unwrap(), 1);
    assert_eq!(cdr_store::queue::retire_copy_only_handoffs(&db).unwrap(), 0);
    assert!(
        unresolved_app_server_fork_handoff_for_source(&db, "source")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        mirrored_thread_id(&db, Some(2)).unwrap().as_deref(),
        Some("source")
    );
    assert_eq!(
        mirrored_thread_id(&db, Some(3)).unwrap().as_deref(),
        Some("child")
    );
    let conn = rusqlite::Connection::open(&db).unwrap();
    let archived: String = conn.query_row("SELECT observed_target_thread_id FROM codex_retired_fork_handoffs WHERE handoff_id='handoff'", [], |r| r.get(0)).unwrap();
    assert_eq!(archived, "child");
}

#[test]
fn ambiguous_request_hold_is_not_retired_or_retargeted() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "source", "project", "Original", 1, 2, 1.0).unwrap();
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "handoff",
            ambiguous_job_id: None,
            source_thread_id: "source",
            expected_generation: 1,
            quarantine_reason: "test",
        },
    )
    .unwrap();
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute(
        "UPDATE codex_thread_fork_handoffs SET ambiguous_job_id='unknown-start'",
        [],
    )
    .unwrap();
    assert_eq!(cdr_store::queue::retire_copy_only_handoffs(&db).unwrap(), 0);
    assert!(
        unresolved_app_server_fork_handoff_for_source(&db, "source")
            .unwrap()
            .is_some()
    );
    assert_eq!(
        mirrored_thread_id(&db, Some(2)).unwrap().as_deref(),
        Some("source")
    );
}

#[test]
fn exact_mode_does_not_redirect_intake_through_a_retained_completed_handoff() {
    use cdr_store::{
        prompt_intake::{NewPromptIntake, admit_prompt_intake},
        queue::complete_app_server_fork_handoff,
    };
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "source", "project", "Original", 1, 2, 1.0).unwrap();
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "handoff",
            ambiguous_job_id: None,
            source_thread_id: "source",
            expected_generation: 1,
            quarantine_reason: "test",
        },
    )
    .unwrap();
    complete_app_server_fork_handoff(&db, "handoff", "child", 1).unwrap();
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute(
            "UPDATE codex_thread_fork_handoffs SET ambiguous_job_id='held-job'",
            [],
        )
        .unwrap();
    assert_eq!(cdr_store::queue::retire_copy_only_handoffs(&db).unwrap(), 0);
    let admitted = admit_prompt_intake(
        &db,
        NewPromptIntake {
            job_id: "request",
            target_thread_id: "source",
            channel_id: 4,
            owner_user_id: Some(5),
            discord_message_id: None,
            raw_prompt: "keep source",
            auto_queue_when_busy: true,
            require_current_mirror: false,
            created_at: 2.0,
        },
    )
    .unwrap();
    assert_eq!(admitted.intake.target_thread_id, "source");
}
