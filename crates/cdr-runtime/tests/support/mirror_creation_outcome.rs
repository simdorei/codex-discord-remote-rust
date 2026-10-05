use super::*;
use std::time::Duration;
use tokio::sync::Notify;

fn retry_sync(
    temp: &tempfile::TempDir,
    remote: &Arc<Remote>,
    original: Arc<MirrorSynchronizer>,
    cold: bool,
) -> Arc<MirrorSynchronizer> {
    if cold {
        Arc::new(MirrorSynchronizer::new(
            temp.path().join("state.sqlite"),
            temp.path().join("mirror.sqlite"),
            remote.clone(),
            Some(1),
        ))
    } else {
        original
    }
}

fn assert_one_unmapped_creation(temp: &tempfile::TempDir, remote: &Remote) {
    let state = remote.0.lock().unwrap();
    assert_eq!(state.creates, 1, "unknown creation must not be replayed");
    assert!(
        state.channels.contains_key(&1001),
        "unknown room must not be deleted"
    );
    assert_eq!(
        thread_channels(&temp.path().join("mirror.sqlite"), "thread-a").unwrap(),
        Some((20, 30)),
        "unknown creation must not authorize remapping"
    );
}

async fn lost_response(cold: bool) {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    let sync = Arc::new(sync);
    {
        let mut state = remote.0.lock().unwrap();
        state.channels.remove(&30);
        state.lose_create_response = Some(ChannelType::PublicThread);
    }
    assert!(sync.sync(99, Some(1)).await.is_err());
    assert_one_unmapped_creation(&temp, &remote);
    remote.0.lock().unwrap().lose_create_response = None;
    let sync = retry_sync(&temp, &remote, sync, cold);
    let retry = sync.sync(99, Some(1)).await;
    assert_one_unmapped_creation(&temp, &remote);
    assert!(
        retry.is_err(),
        "lost response remains unknown, not guessed from inventory"
    );
}

async fn cancelled_after_create(cold: bool) {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    let sync = Arc::new(sync);
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    {
        let mut state = remote.0.lock().unwrap();
        state.channels.remove(&30);
        state.pause_create = Some((ChannelType::PublicThread, started.clone(), release));
    }
    let running = sync.clone();
    let task = tokio::spawn(async move { running.sync(99, Some(1)).await });
    tokio::time::timeout(Duration::from_secs(10), started.notified())
        .await
        .expect("fixture must reach the create side effect");
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_one_unmapped_creation(&temp, &remote);
    remote.0.lock().unwrap().pause_create = None;
    let sync = retry_sync(&temp, &remote, sync, cold);
    let retry = sync.sync(99, Some(1)).await;
    assert_one_unmapped_creation(&temp, &remote);
    assert!(
        retry.is_err(),
        "cancelled create must retain unknown custody"
    );
}

async fn mapping_write_failure(cold: bool) {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    let sync = Arc::new(sync);
    let path = temp.path().join("mirror.sqlite");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_mirror_mapping BEFORE UPDATE ON mirror_threads
         WHEN NEW.codex_thread_id='thread-a' AND NEW.discord_thread_id <> OLD.discord_thread_id
         BEGIN SELECT RAISE(ABORT, 'injected mapping write failure'); END;",
    )
    .unwrap();
    remote.0.lock().unwrap().channels.remove(&30);
    assert!(sync.sync(99, Some(1)).await.is_err());
    assert_one_unmapped_creation(&temp, &remote);
    db.execute_batch("DROP TRIGGER reject_mirror_mapping;")
        .unwrap();
    let sync = retry_sync(&temp, &remote, sync, cold);
    sync.sync(99, Some(1)).await.unwrap();
    assert_eq!(
        remote.0.lock().unwrap().creates,
        1,
        "reuse the confirmed room ID"
    );
    assert_eq!(
        thread_channels(&path, "thread-a").unwrap(),
        Some((20, 1001))
    );
}

#[tokio::test]
async fn mirror_creation_lost_response_same_coordinator() {
    lost_response(false).await;
}

#[tokio::test]
async fn mirror_creation_lost_response_cold_coordinator() {
    lost_response(true).await;
}

#[tokio::test]
async fn mirror_creation_cancelled_same_coordinator() {
    cancelled_after_create(false).await;
}

#[tokio::test]
async fn mirror_creation_cancelled_cold_coordinator() {
    cancelled_after_create(true).await;
}

#[tokio::test]
async fn mirror_creation_mapping_failure_same_coordinator() {
    mapping_write_failure(false).await;
}

#[tokio::test]
async fn mirror_creation_mapping_failure_cold_coordinator() {
    mapping_write_failure(true).await;
}

#[tokio::test]
async fn mirror_creation_unknown_cannot_bypass_when_old_room_reappears() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    {
        let mut state = remote.0.lock().unwrap();
        state.channels.remove(&30);
        state.lose_create_response = Some(ChannelType::PublicThread);
    }
    assert!(sync.sync(99, Some(1)).await.is_err());
    {
        let mut state = remote.0.lock().unwrap();
        state.lose_create_response = None;
        state.channels.insert(
            30,
            channel(30, Some(20), ChannelType::PublicThread, "restored"),
        );
    }
    assert!(sync.sync(99, Some(1)).await.is_err());
    assert_one_unmapped_creation(&temp, &remote);
}

#[tokio::test]
async fn mirror_creation_ignored_intent_write_dispatches_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    let db = Connection::open(temp.path().join("mirror.sqlite")).unwrap();
    db.execute_batch(
        "CREATE TRIGGER ignore_creation BEFORE INSERT ON cdr_mirror_thread_creations
        BEGIN SELECT RAISE(IGNORE); END;",
    )
    .unwrap();
    remote.0.lock().unwrap().channels.remove(&30);
    assert!(sync.sync(99, Some(1)).await.is_err());
    assert_eq!(remote.0.lock().unwrap().creates, 0);
    db.execute_batch("DROP TRIGGER ignore_creation;").unwrap();
    sync.sync(99, Some(1)).await.unwrap();
    assert_eq!(remote.0.lock().unwrap().creates, 1);
}

#[tokio::test]
async fn mirror_creation_confirmation_failure_stays_unknown_after_reconstruction() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    let db = Connection::open(temp.path().join("mirror.sqlite")).unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_confirmation BEFORE UPDATE ON cdr_mirror_thread_creations
        BEGIN SELECT RAISE(ABORT,'injected confirmation failure'); END;",
    )
    .unwrap();
    remote.0.lock().unwrap().channels.remove(&30);
    assert!(sync.sync(99, Some(1)).await.is_err());
    db.execute_batch("DROP TRIGGER reject_confirmation;")
        .unwrap();
    let sync = retry_sync(&temp, &remote, Arc::new(sync), true);
    assert!(sync.sync(99, Some(1)).await.is_err());
    assert_one_unmapped_creation(&temp, &remote);
}

#[tokio::test]
async fn mirror_creation_new_thread_lost_response_is_not_replayed() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    remote.0.lock().unwrap().lose_create_response = Some(ChannelType::PublicThread);
    assert!(
        sync.link_new_thread(20, "thread-new", "new prompt", Some("C:/repos/alpha"))
            .await
            .is_err()
    );
    remote.0.lock().unwrap().lose_create_response = None;
    let sync = retry_sync(&temp, &remote, Arc::new(sync), true);
    assert!(
        sync.link_new_thread(20, "thread-new", "new prompt", Some("C:/repos/alpha"))
            .await
            .is_err()
    );
    assert_eq!(remote.0.lock().unwrap().creates, 1);
    assert_eq!(
        thread_channels(&temp.path().join("mirror.sqlite"), "thread-new").unwrap(),
        None
    );
}

async fn confirmed_room_changed(change: fn(&mut RemoteState)) {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    let db = Connection::open(temp.path().join("mirror.sqlite")).unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_mapping BEFORE UPDATE ON mirror_threads
        WHEN NEW.codex_thread_id='thread-a' AND NEW.discord_thread_id<>OLD.discord_thread_id
        BEGIN SELECT RAISE(ABORT,'injected mapping failure'); END;",
    )
    .unwrap();
    remote.0.lock().unwrap().channels.remove(&30);
    assert!(sync.sync(99, Some(1)).await.is_err());
    db.execute_batch("DROP TRIGGER reject_mapping;").unwrap();
    change(&mut remote.0.lock().unwrap());
    let sync = retry_sync(&temp, &remote, Arc::new(sync), true);
    assert!(sync.sync(99, Some(1)).await.is_err());
    assert_eq!(remote.0.lock().unwrap().creates, 1);
    assert_eq!(
        thread_channels(&temp.path().join("mirror.sqlite"), "thread-a").unwrap(),
        Some((20, 30))
    );
}

#[tokio::test]
async fn mirror_creation_confirmed_room_missing_never_creates_again() {
    confirmed_room_changed(|state| {
        state.channels.remove(&1001);
    })
    .await;
}

#[tokio::test]
async fn mirror_creation_confirmed_room_wrong_parent_never_remaps() {
    confirmed_room_changed(|state| {
        state.channels.get_mut(&1001).unwrap().parent_id = Some(21);
    })
    .await;
}
