use super::*;

#[test]
fn isolated_backup_restores_databases_metadata_index_and_transcript() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    let rollout = paths.archived_sessions.join("old.jsonl");
    seed(&paths, &rollout, true);
    let metadata = [
        &paths.bridge_state,
        &paths.global_state,
        &paths.session_index,
    ];
    let before = metadata
        .iter()
        .map(|p| fs::read(p).unwrap())
        .collect::<Vec<_>>();
    let result = delete_archived_thread(&paths, "thread-old").unwrap();
    // Only this disposable fixture is restored; no live database or application is involved.
    for destination in [
        &paths.state_db,
        &paths.log_db,
        &paths.bridge_state,
        &paths.global_state,
        &paths.session_index,
    ] {
        fs::copy(
            result.backup_dir.join(destination.file_name().unwrap()),
            destination,
        )
        .unwrap();
    }
    fs::copy(result.backup_dir.join("transcript/rollout.jsonl"), &rollout).unwrap();
    assert_eq!(row_count(&paths.state_db, "threads", "thread-old"), 1);
    let state = Connection::open(&paths.state_db).unwrap();
    assert_eq!(
        state
            .query_row("SELECT COUNT(*) FROM thread_spawn_edges", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        Connection::open(&paths.log_db)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM logs WHERE thread_id='thread-old'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    for (path, expected) in metadata.iter().zip(before) {
        assert_eq!(fs::read(path).unwrap(), expected);
    }
    assert_eq!(fs::read(rollout).unwrap(), b"rollout\n");
}

#[test]
fn conflicting_backup_names_never_overwrite_a_recovery_copy_or_delete_source() {
    let temp = tempfile::tempdir().unwrap();
    let mut paths = paths(temp.path());
    let nested = temp.path().join("nested");
    fs::create_dir(&nested).unwrap();
    paths.bridge_state = nested.join("global.json");
    let rollout = paths.archived_sessions.join("old.jsonl");
    seed(&paths, &rollout, true);
    assert!(
        delete_archived_thread(&paths, "thread-old")
            .unwrap_err()
            .to_string()
            .contains("filename collision")
    );
    assert_eq!(row_count(&paths.state_db, "threads", "thread-old"), 1);
    assert!(rollout.exists());
}
#[test]
fn malformed_metadata_is_rejected_before_source_records_are_deleted() {
    for field in ["bridge", "global"] {
        let temp = tempfile::tempdir().unwrap();
        let paths = super::paths(temp.path());
        let rollout = paths.archived_sessions.join("thread-old.jsonl");
        super::seed(&paths, &rollout, true);
        let invalid = if field == "bridge" {
            &paths.bridge_state
        } else {
            &paths.global_state
        };
        std::fs::write(invalid, b"invalid-json").unwrap();
        let error = super::delete_archived_thread(&paths, "thread-old").unwrap_err();
        assert!(error.to_string().contains("JSON"));
        assert_eq!(
            super::row_count(&paths.state_db, "threads", "thread-old"),
            1
        );
        let logs: i64 = rusqlite::Connection::open(&paths.log_db)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM logs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(logs, 1);
        assert_eq!(std::fs::read(&rollout).unwrap(), b"rollout\n");
    }
}
