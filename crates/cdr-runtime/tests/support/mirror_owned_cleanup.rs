use super::*;
#[allow(dead_code)]
#[path = "../../../cdr-store/tests/support/owned_prompt.rs"]
mod owned_prompt;

#[tokio::test]
async fn mc_1_public_handoff_final_delivery_then_archive_allows_sync_and_keeps_history() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    let db = temp.path().join("mirror.sqlite");
    owned_prompt::settled_for(&db, "thread-old", "C:/repos/old", 21);
    let before = cdr_store::ingress::get(&db, "message:501")
        .unwrap()
        .unwrap();
    assert_eq!(
        Connection::open(temp.path().join("state.sqlite"))
            .unwrap()
            .query_row(
                "SELECT archived FROM threads WHERE id='thread-old'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    let result = sync.sync(99, None).await.unwrap();
    assert_eq!(result.archived, 1);
    assert!(!remote.0.lock().unwrap().channels.contains_key(&31));
    assert!(thread_channels(&db, "thread-old").unwrap().is_none());
    assert_eq!(
        cdr_store::ingress::get(&db, "message:501")
            .unwrap()
            .unwrap(),
        before
    );
    assert_eq!(sync.sync(99, None).await.unwrap().archived, 0);
}

#[tokio::test]
async fn mc_4_real_precheck_preserves_pending_request_and_actual_blocked_room() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    let db = temp.path().join("mirror.sqlite");
    owned_prompt::queued_for(&db, "thread-old", "C:/repos/old", 21);
    let error = sync.sync(99, None).await.unwrap_err();
    assert!(matches!(
        error,
        MirrorSyncError::CleanupProtected {
            channel: 31,
            reason: "queued requests"
        }
    ));
    assert!(remote.0.lock().unwrap().channels.contains_key(&31));
    assert!(thread_channels(&db, "thread-old").unwrap().is_some());
    assert_eq!(cdr_store::room_cleanup::phase(&db, 31).unwrap(), None);
    assert_eq!(cdr_store::queue::list(&db).unwrap().len(), 1);
}

#[tokio::test]
async fn mc_5_missing_room_race_is_typed_and_keeps_partial_sync_changes() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    let db = temp.path().join("mirror.sqlite");
    owned_prompt::settled_for(&db, "thread-old", "C:/repos/old", 21);
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    remote.0.lock().unwrap().pause_channel = Some((31, started.clone(), release.clone()));
    let worker = tokio::spawn(async move { sync.sync(99, None).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), started.notified())
        .await
        .unwrap();
    // A different client removes the room after read-only precheck, while a
    // request arrives. The final transaction must not retire its live mapping.
    remote.0.lock().unwrap().channels.remove(&31);
    let mut pending = owned_prompt::request();
    pending.ingress_id = "message:502".into();
    pending.event_id = Some(502);
    pending.source_message_id = Some(502);
    pending.target_thread_id = Some("thread-old".into());
    cdr_store::ingress::admit(&db, &pending).unwrap();
    release.notify_one();
    let error = worker.await.unwrap().unwrap_err();
    assert!(matches!(
        error,
        MirrorSyncError::CleanupProtected {
            channel: 31,
            reason: "ingress"
        }
    ));
    assert!(error.to_string().contains("earlier sync changes"));
    assert!(!remote.0.lock().unwrap().channels.contains_key(&31));
    assert_eq!(
        remote.0.lock().unwrap().channels[&30].name,
        "앱에서 정한 이름"
    );
    assert!(thread_channels(&db, "thread-old").unwrap().is_some());
    assert_eq!(cdr_store::room_cleanup::phase(&db, 31).unwrap(), None);
    assert_eq!(
        cdr_store::ingress::get(&db, "message:502")
            .unwrap()
            .unwrap()
            .state,
        "staged"
    );
}
