use super::*;

#[tokio::test]
async fn project_reassignment_during_room_create_does_not_commit_a_stale_mapping() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_project(&db, "C:/old", "old", 99, 1.0, |a, b| a == b).unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let remote = Arc::new(Remote {
        pause_create: Some((entered.clone(), resume.clone())),
        ..Default::default()
    });
    let sync = cdr_runtime::mirror_sync::MirrorSynchronizer::new(
        temp.path().join("state.sqlite"),
        db.clone(),
        remote.clone(),
        Some(1),
    );
    let change = async {
        entered.notified().await;
        cdr_store::schema::open_initialized(&db)
            .unwrap()
            .execute(
                "UPDATE mirror_projects SET project_key='C:/different' WHERE discord_channel_id=99",
                [],
            )
            .unwrap();
        resume.notify_one();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(
            sync.link_new_thread(99, "new-thread", "new", Some("C:/old")),
            change
        )
    })
    .await
    .unwrap();
    assert!(
        result.is_err(),
        "new room must not be attached after its project changed"
    );
    assert!(mirrored_thread_id(&db, Some(100)).unwrap().is_none());
    assert_eq!(
        *remote.creates.lock().unwrap(),
        1,
        "keep uncertain created room, do not delete or retry"
    );
}
