use super::*;

fn absent(temp: &tempfile::TempDir) {
    Connection::open(temp.path().join("state.sqlite"))
        .unwrap()
        .execute("DELETE FROM threads WHERE id='thread-old'", [])
        .unwrap();
}

#[tokio::test]
async fn exact_cleanup_only_retires_approved_room() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    absent(&temp);
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "also-absent", "C:/repos/old", "keep", 21, 32, 1.0).unwrap();
    remote
        .0
        .lock()
        .unwrap()
        .channels
        .insert(32, channel(32, Some(21), ChannelType::PublicThread, "keep"));
    sync.retire_exact_absent("thread-old", 31, 21)
        .await
        .unwrap();
    assert!(thread_channels(&db, "thread-old").unwrap().is_none());
    assert!(thread_channels(&db, "also-absent").unwrap().is_some());
    {
        let state = remote.0.lock().unwrap();
        assert!(!state.channels.contains_key(&31));
        assert!(state.channels.contains_key(&32));
        assert_eq!(state.creates, 0);
    }
    // A crash after success but before the worker saved its phase must not DELETE again.
    sync.retire_exact_absent("thread-old", 31, 21)
        .await
        .unwrap();
}

#[tokio::test]
async fn confirmed_delete_resumes_only_mapping_retirement() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    absent(&temp);
    let db = temp.path().join("mirror.sqlite");
    let token = cdr_store::room_cleanup::begin(&db, 31, Some("thread-old"), 2.0).unwrap();
    cdr_store::room_cleanup::complete(&db, 31, &token).unwrap();
    // A contradictory remote room must not be deleted again.
    assert!(
        sync.retire_exact_absent("thread-old", 31, 21)
            .await
            .is_err()
    );
    assert!(remote.0.lock().unwrap().channels.contains_key(&31));
    remote.0.lock().unwrap().channels.remove(&31);
    sync.retire_exact_absent("thread-old", 31, 21)
        .await
        .unwrap();
    assert!(thread_channels(&db, "thread-old").unwrap().is_none());
}

#[tokio::test]
async fn exact_cleanup_rejects_wrong_room_parent_active_and_shared() {
    for case in ["room", "parent", "active", "shared", "inventory"] {
        let temp = tempfile::tempdir().unwrap();
        let (sync, remote) = fixture(&temp);
        absent(&temp);
        let db = temp.path().join("mirror.sqlite");
        if case == "active" {
            Connection::open(temp.path().join("state.sqlite")).unwrap().execute(
                "INSERT INTO threads (id,title,cwd,updated_at,rollout_path,archived,source,thread_source) VALUES ('thread-old','restored','C:/repos/old',99,'missing.jsonl',0,'vscode','user')", []
            ).unwrap();
        }
        if case == "shared" {
            upsert_thread(&db, "also-absent", "C:/repos/old", "shared", 21, 31, 1.0).unwrap();
        }
        if case == "inventory" {
            Connection::open(temp.path().join("state.sqlite"))
                .unwrap()
                .execute("ALTER TABLE threads RENAME TO broken", [])
                .unwrap();
        }
        let room = if case == "room" { 32 } else { 31 };
        let parent = if case == "parent" { 22 } else { 21 };
        assert!(
            sync.retire_exact_absent("thread-old", room, parent)
                .await
                .is_err(),
            "{case}"
        );
        assert!(
            thread_channels(&db, "thread-old").unwrap().is_some(),
            "{case}"
        );
        assert!(
            remote.0.lock().unwrap().channels.contains_key(&31),
            "{case}"
        );
        assert_eq!(
            cdr_store::room_cleanup::phase(&db, 31).unwrap(),
            None,
            "{case}"
        );
    }
}

#[tokio::test]
async fn exact_cleanup_preserves_uncertain_outcome() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    absent(&temp);
    remote.0.lock().unwrap().lose_delete_response = Some(31);
    let error = sync
        .retire_exact_absent("thread-old", 31, 21)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("unconfirmed"));
    assert!(
        sync.retire_exact_absent("thread-old", 31, 21)
            .await
            .is_err()
    );
    let db = temp.path().join("mirror.sqlite");
    assert!(thread_channels(&db, "thread-old").unwrap().is_some());
    assert_eq!(
        cdr_store::room_cleanup::phase(&db, 31).unwrap().as_deref(),
        Some("deleting")
    );
}

#[tokio::test]
async fn exact_cleanup_preserves_pending_request() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    absent(&temp);
    let db = temp.path().join("mirror.sqlite");
    cdr_store::queue::enqueue(
        &db,
        cdr_store::queue::NewQueueJob {
            job_id: "pending-exact",
            target_thread_id: "thread-old",
            channel_id: 31,
            owner_user_id: None,
            discord_message_id: None,
            app_server_generation: 1,
            prompt: "preserve",
            queued: true,
            ack_sent: false,
            created_at: 1.0,
        },
    )
    .unwrap();
    let error = sync
        .retire_exact_absent("thread-old", 31, 21)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("queued requests"));
    assert!(thread_channels(&db, "thread-old").unwrap().is_some());
    assert!(remote.0.lock().unwrap().channels.contains_key(&31));
    assert_eq!(cdr_store::room_cleanup::phase(&db, 31).unwrap(), None);
}

#[tokio::test]
async fn exact_cleanup_rechecks_identity_after_lookup() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    absent(&temp);
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    remote.0.lock().unwrap().pause_channel = Some((31, started.clone(), release.clone()));
    let task = tokio::spawn(async move { sync.retire_exact_absent("thread-old", 31, 21).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), started.notified())
        .await
        .unwrap();
    Connection::open(temp.path().join("state.sqlite")).unwrap().execute(
        "INSERT INTO threads (id,title,cwd,updated_at,rollout_path,archived,source,thread_source) VALUES ('thread-old','restored','C:/repos/old',99,'missing.jsonl',0,'vscode','user')", []
    ).unwrap();
    release.notify_one();
    assert!(task.await.unwrap().is_err());
    assert!(remote.0.lock().unwrap().channels.contains_key(&31));
}
