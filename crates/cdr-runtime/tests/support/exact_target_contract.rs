use super::*;
#[path = "target_command_inputs.rs"]
mod inputs;

#[tokio::test]
async fn explicit_blank_slash_ref_never_executes_or_changes_selected_target() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state.sqlite");
    Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("../fixtures/action_state.sql"))
        .unwrap();
    let db = temp.path().join("mirror.sqlite");
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("thread-b")).unwrap();
    cdr_store::mapping::upsert_thread(&db, "thread-a", "project", "a", 98, 99, 1.0).unwrap();
    let executor = ActionExecutor::new(
        state,
        db.clone(),
        bridge.clone(),
        Arc::new(QueueCoordinator::new(db.clone(), Arc::new(ReadOnlyBackend))),
    );
    let mut calls = 0;
    for name in ["status", "settings", "retract"] {
        let planned = inputs::slash_reference(name, Some(" \t "));
        if let Ok(action) = planned {
            calls += 1;
            let _ = executor.execute(action, 99, 20).await;
        }
    }
    assert_eq!(
        calls, 0,
        "explicit invalid ref must fail before Action execution"
    );
    assert_eq!(
        bridge.selected_thread_id().unwrap().as_deref(),
        Some("thread-b")
    );
    assert_eq!(
        cdr_store::mapping::mirrored_thread_id(&db, Some(99))
            .unwrap()
            .as_deref(),
        Some("thread-a")
    );
    let omitted = executor
        .execute(inputs::slash_reference("status", None).unwrap(), 99, 20)
        .await
        .unwrap();
    assert!(omitted.text.contains("thread-a"));
}

#[tokio::test]
async fn displayed_collision_aliases_round_trip_through_prefix_slash_and_action() {
    for workspace in [
        "abcd",
        "other",
        "next",
        "1",
        "thread-a",
        " 1",
        " other",
        " next",
        "\u{2003}1",
        "my project",
        "x|y",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state.sqlite");
        let connection = Connection::open(&state).unwrap();
        connection
            .execute_batch(include_str!("../fixtures/action_state.sql"))
            .unwrap();
        connection
            .execute(
                "UPDATE threads SET id='abcd0000-1111-2222-3333-444444444444' WHERE id='thread-a'",
                [],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE threads SET cwd=? WHERE id='thread-b'",
                [format!("C:/repos/{workspace}")],
            )
            .unwrap();
        let db = temp.path().join("mirror.sqlite");
        let executor = ActionExecutor::new(
            state,
            db.clone(),
            Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
            Arc::new(QueueCoordinator::new(db, Arc::new(ReadOnlyBackend))),
        );
        let list = executor
            .execute(inputs::prefix("!list"), 99, 20)
            .await
            .unwrap();
        for row in list.text.lines() {
            let columns = row.split('|').map(str::trim).collect::<Vec<_>>();
            let (alias, id) = (columns[1], columns[2]);
            let selected = executor
                .execute(inputs::prefix(&format!("!use {alias}")), 99, 20)
                .await
                .unwrap();
            assert!(selected.text.contains(id), "{}", selected.text);
            let status = executor
                .execute(inputs::slash_status(alias), 99, 20)
                .await
                .unwrap();
            assert!(status.text.contains(id), "{}", status.text);
        }
    }
}

#[tokio::test]
async fn exact_old_archived_id_supports_preview_and_explicit_confirmation() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state.sqlite");
    let db = Connection::open(&state).unwrap();
    db.execute_batch(include_str!("../fixtures/action_state.sql"))
        .unwrap();
    db.execute_batch(
        "CREATE TABLE thread_spawn_edges(parent_thread_id TEXT,child_thread_id TEXT);",
    )
    .unwrap();
    for index in 0..105 {
        db.execute("INSERT INTO threads SELECT ?1,title,cwd,?2,rollout_path,model,reasoning_effort,tokens_used,1,?2,source,thread_source FROM threads WHERE id='thread-old'",(format!("archive-{index}"),1000+index)).unwrap();
    }
    let archive = temp.path().join("archived_sessions");
    std::fs::create_dir(&archive).unwrap();
    let rollout = archive.join("old.jsonl");
    std::fs::write(&rollout, b"preserved in backup").unwrap();
    db.execute(
        "UPDATE threads SET rollout_path=? WHERE id='thread-old'",
        [rollout.to_string_lossy().as_ref()],
    )
    .unwrap();
    let mirror = temp.path().join("mirror.sqlite");
    let executor = ActionExecutor::new(
        state,
        mirror.clone(),
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        Arc::new(QueueCoordinator::new(mirror, Arc::new(ReadOnlyBackend))),
    );
    let preview = executor
        .execute(inputs::prefix("!delete_archive thread-old"), 99, 20)
        .await
        .unwrap();
    assert!(preview.text.contains("thread_id: thread-old"));
    assert!(rollout.exists());
    assert!(!temp.path().join("maintenance_backups").exists());
    let deleted = executor
        .execute(inputs::prefix("!confirm_delete_archive thread-old"), 99, 20)
        .await
        .unwrap();
    assert!(deleted.text.contains("thread_id: thread-old"));
    assert!(!rollout.exists());
    let remaining: i64 = db
        .query_row("SELECT COUNT(*) FROM threads WHERE archived=1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(remaining, 105);
}

#[tokio::test]
async fn exact_and_mirrored_old_ids_are_not_limited_by_recent_list_size() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state.sqlite");
    let db = Connection::open(&state).unwrap();
    db.execute_batch(include_str!("../fixtures/action_state.sql"))
        .unwrap();
    for index in 0..105 {
        db.execute("INSERT INTO threads SELECT ?1,title,cwd,?2,rollout_path,model,reasoning_effort,tokens_used,0,0,source,thread_source FROM threads WHERE id='thread-b'",(format!("newer-{index}"),1000+index)).unwrap();
    }
    let mirror = temp.path().join("mirror.sqlite");
    cdr_store::mapping::upsert_thread(&mirror, "thread-a", "alpha", "First", 90, 10, 1.0).unwrap();
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("thread-b")).unwrap();
    let executor = ActionExecutor::new(
        state,
        mirror.clone(),
        bridge,
        Arc::new(QueueCoordinator::new(mirror, Arc::new(ReadOnlyBackend))),
    );
    for action in [
        inputs::prefix("!status thread-a"),
        inputs::prefix("!status"),
        inputs::slash_status("thread-a"),
    ] {
        let result = executor.execute(action, 10, 20).await.unwrap();
        assert!(result.text.contains("thread_id: thread-a"));
    }
    let result = executor
        .execute(
            CommandAction::Use {
                reference: "thread-a".into(),
            },
            99,
            20,
        )
        .await
        .unwrap();
    assert!(result.text.contains("thread_id: thread-a"));
    let error = executor
        .execute(
            CommandAction::Status {
                reference: Some("missing-exact-id".into()),
            },
            10,
            20,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("missing-exact-id"));
    let listed = executor
        .execute(CommandAction::List { limit: 1 }, 99, 20)
        .await
        .unwrap();
    assert!(listed.text.contains("beta:1"), "{}", listed.text);
    assert_eq!(listed.text.lines().count(), 1);
    let selected = executor
        .execute(
            CommandAction::Use {
                reference: "beta:1".into(),
            },
            99,
            20,
        )
        .await
        .unwrap();
    assert!(selected.text.contains("newer-104"));
    let unmapped = executor
        .execute(CommandAction::Where, 99, 20)
        .await
        .unwrap();
    assert!(unmapped.text.contains("unmapped; using global selected"));
}
