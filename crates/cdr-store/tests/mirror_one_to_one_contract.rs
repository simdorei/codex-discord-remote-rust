use cdr_store::mapping::{
    MirrorThreadUpdate, commit_thread_sync, mirrored_thread_id, upsert_thread,
};

#[test]
fn duplicate_room_is_reported_instead_of_selecting_an_arbitrary_thread() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "a", "p", "A", 1, 2, 1.0).unwrap();
    upsert_thread(&db, "b", "p", "B", 1, 2, 1.0).unwrap();
    assert!(mirrored_thread_id(&db, Some(2)).is_err());
}

#[test]
fn sync_cannot_attach_a_second_codex_id_to_an_owned_room() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "a", "p", "A", 1, 2, 1.0).unwrap();
    assert!(
        commit_thread_sync(
            &db,
            MirrorThreadUpdate {
                thread_id: "b",
                project_key: "p",
                title: "B",
                parent_id: 1,
                channel_id: 2,
                now: 2.0,
            },
            None
        )
        .is_err()
    );
    assert_eq!(
        mirrored_thread_id(&db, Some(2)).unwrap().as_deref(),
        Some("a")
    );
}
