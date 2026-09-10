use super::*;

#[test]
fn changed_rollout_identity_rolls_back_row_and_edge_deletion() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let connection = Connection::open(&db).unwrap();
    connection.execute_batch("CREATE TABLE threads (id TEXT, archived INTEGER, rollout_path TEXT); CREATE TABLE thread_spawn_edges (parent_thread_id TEXT,child_thread_id TEXT); INSERT INTO threads VALUES ('old',1,'replacement.jsonl'); INSERT INTO thread_spawn_edges VALUES ('parent','old');").unwrap();
    let paths = ArchiveDeletePaths {
        state_db: db,
        log_db: temp.path().join("logs.sqlite"),
        global_state: temp.path().join("global.json"),
        bridge_state: temp.path().join("bridge.json"),
        session_index: temp.path().join("index.jsonl"),
        archived_sessions: temp.path().join("archived_sessions"),
        backup_root: temp.path().join("backups"),
    };
    assert!(matches!(
        delete_state_row(
            &paths,
            "old",
            Path::new("original.jsonl"),
            &paths.backup_root
        ),
        Err(ArchiveDeleteError::ConcurrentMutation)
    ));
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM threads", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM thread_spawn_edges", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn metadata_changed_after_precheck_must_not_be_deleted_without_a_matching_backup() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("global.json");
    let backup = temp.path().join("backup");
    fs::create_dir(&backup).unwrap();
    fs::write(&source, b"{}").unwrap();
    fs::copy(&source, backup.join("global.json")).unwrap();
    verification::unchanged_optional_file(&source, &backup).unwrap();
    let late = br#"{"queued-follow-ups":{"old":["new request"]}}"#;
    fs::write(&source, late).unwrap();
    assert!(scrub::scrub_json_state(&source, "old", false, &backup).is_err());
    assert_eq!(fs::read(&source).unwrap(), late);
}

#[test]
fn index_changes_and_new_metadata_without_backup_are_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let backup = temp.path().join("backup");
    fs::create_dir(&backup).unwrap();
    let source = temp.path().join("index.jsonl");
    fs::write(&source, b"{\"id\":\"old\"}\n").unwrap();
    fs::copy(&source, backup.join("index.jsonl")).unwrap();
    let late = b"{\"id\":\"old\",\"name\":\"new name\"}\n";
    fs::write(&source, late).unwrap();
    assert!(scrub::scrub_session_index(&source, "old", &backup).is_err());
    assert_eq!(fs::read(&source).unwrap(), late);
    let new_json = temp.path().join("new.json");
    fs::write(&new_json, b"{\"selected_thread_id\":\"old\"}").unwrap();
    assert!(scrub::scrub_json_state(&new_json, "old", true, &backup).is_err());
    assert_eq!(
        fs::read(&new_json).unwrap(),
        b"{\"selected_thread_id\":\"old\"}"
    );
}

#[test]
fn whole_delete_stops_at_exact_stage_when_metadata_changes_after_backup_check() {
    for (field, expected_stage, late) in [
        (
            "bridge",
            "log deletion",
            br#"{"selected_thread_id":"old","recent_ui_thread":{"thread_id":"old","new":"keep"}}"#
                .as_slice(),
        ),
        (
            "global",
            "bridge state update",
            br#"{"queued-follow-ups":{"old":["late request"]}}"#.as_slice(),
        ),
        (
            "index",
            "global state update",
            b"{\"id\":\"old\",\"name\":\"late name\"}\n".as_slice(),
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let paths = ArchiveDeletePaths {
            state_db: root.join("state.sqlite"),
            log_db: root.join("logs.sqlite"),
            global_state: root.join("global.json"),
            bridge_state: root.join("bridge.json"),
            session_index: root.join("index.jsonl"),
            archived_sessions: root.join("archived_sessions"),
            backup_root: root.join("backups"),
        };
        fs::create_dir(&paths.archived_sessions).unwrap();
        let rollout = paths.archived_sessions.join("old.jsonl");
        fs::write(&rollout, b"original transcript").unwrap();
        let db = Connection::open(&paths.state_db).unwrap();
        db.execute_batch("CREATE TABLE threads(id TEXT,archived INTEGER,rollout_path TEXT);CREATE TABLE thread_spawn_edges(parent_thread_id TEXT,child_thread_id TEXT);").unwrap();
        db.execute(
            "INSERT INTO threads VALUES('old',1,?)",
            [rollout.to_string_lossy().as_ref()],
        )
        .unwrap();
        fs::write(&paths.bridge_state, b"{}").unwrap();
        fs::write(&paths.global_state, b"{}").unwrap();
        fs::write(&paths.session_index, b"{\"id\":\"old\"}\n").unwrap();
        let changed = match field {
            "bridge" => &paths.bridge_state,
            "global" => &paths.global_state,
            _ => &paths.session_index,
        };
        let error = delete_with_metadata_observer(&paths, "old", |path| {
            if path == changed {
                fs::write(path, late).unwrap();
            }
        })
        .unwrap_err();
        let ArchiveDeleteError::Partial {
            last_completed_stage,
            backup_dir,
            source,
        } = error
        else {
            panic!("partial failure required")
        };
        assert_eq!(last_completed_stage, expected_stage);
        assert!(matches!(*source, ArchiveDeleteError::ConcurrentMutation));
        assert_eq!(fs::read(changed).unwrap(), late);
        assert_eq!(fs::read(&rollout).unwrap(), b"original transcript");
        assert_eq!(
            fs::read(backup_dir.join("transcript/rollout.jsonl")).unwrap(),
            b"original transcript"
        );
    }
}
