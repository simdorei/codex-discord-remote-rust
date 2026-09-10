use super::*;

#[tokio::test]
async fn project_relations_are_diagnosed_without_mutation_through_real_inputs() {
    for (projects, expected) in [
        (vec![], "missing_project_mapping: 2"),
        (vec![("C:/Repo", 91)], "project_parent_mismatch: 2"),
        (
            vec![("C:/Repo", 90), ("c:\\repo\\", 91)],
            "ambiguous_project_mapping: 2",
        ),
        (
            vec![("C:/Repo", 90), ("C:/Other", 90)],
            "duplicate_project_channels: 1",
        ),
        (vec![("C:/Repo", 90)], "status: ok"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state.sqlite");
        let connection = rusqlite::Connection::open(&state).unwrap();
        connection
            .execute_batch(include_str!("../fixtures/action_state.sql"))
            .unwrap();
        let rollout = tempfile::NamedTempFile::new_in(temp.path()).unwrap();
        connection
            .execute(
                "UPDATE threads SET rollout_path=?1 WHERE archived=0",
                [rollout.path().to_str().unwrap()],
            )
            .unwrap();
        let db = temp.path().join("mirror.sqlite");
        for (thread, room) in [("thread-a", 100), ("thread-b", 101)] {
            upsert_thread(&db, thread, "c:\\repo\\", thread, 90, room, 1.0).unwrap();
        }
        for (key, room) in projects {
            cdr_store::mapping::upsert_project(&db, key, key, room, 1.0, |a, b| a == b).unwrap();
        }
        let executor = ActionExecutor::new(
            state,
            db.clone(),
            Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
            Arc::new(QueueCoordinator::new(
                db.clone(),
                Arc::new(support::FakeBackend::default()),
            )),
        );
        executor
            .set_mirror_transport(Arc::new(ReadOnlyRemote), Some(1))
            .unwrap();
        let before = std::fs::read(&db).unwrap();
        for planned in [
            action("!mirror check"),
            action("!mirror check 1"),
            action("!mirror list"),
            action("!mirror list 1"),
            slash_check(),
        ] {
            let result = executor.execute(planned, 90, 20).await.unwrap();
            assert!(
                result.text.contains(expected),
                "expected {expected}: {}",
                result.text
            );
            if expected != "status: ok" {
                assert!(result.text.contains("status: issues_found"));
            }
            assert_eq!(std::fs::read(&db).unwrap(), before);
        }
    }
}
