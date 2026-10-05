use super::*;
use std::time::Duration;
use tokio::sync::Notify;

fn reconstructed(temp: &tempfile::TempDir, remote: &Arc<Remote>) -> Arc<MirrorSynchronizer> {
    Arc::new(MirrorSynchronizer::new(
        temp.path().join("state.sqlite"),
        temp.path().join("mirror.sqlite"),
        remote.clone(),
        Some(1),
    ))
}

fn project_id(temp: &tempfile::TempDir) -> i64 {
    cdr_store::mapping::find_project(
        &temp.path().join("mirror.sqlite"),
        Some("C:/repos/alpha"),
        |left, right| {
            cdr_codex_state::normalize_workspace_path(left)
                == cdr_codex_state::normalize_workspace_path(right)
        },
    )
    .unwrap()
    .unwrap()
    .channel_id
}

fn foreign_project(remote: &Remote) {
    let mut foreign = channel(40, Some(10), ChannelType::GuildText, "alpha");
    foreign.topic = Some("Codex project mirror: alpha".into());
    remote.0.lock().unwrap().channels.insert(40, foreign);
}

async fn unknown(kind: ChannelType, cancelled: bool, cold: bool) {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    let sync = Arc::new(sync);
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    {
        let mut state = remote.0.lock().unwrap();
        state
            .channels
            .remove(&if kind == ChannelType::GuildCategory {
                10
            } else {
                20
            });
        if cancelled {
            state.pause_create = Some((kind, started.clone(), release));
        } else {
            state.lose_create_response = Some(kind);
        }
    }
    if cancelled {
        let running = sync.clone();
        let task = tokio::spawn(async move { running.sync(99, Some(1)).await });
        tokio::time::timeout(Duration::from_secs(10), started.notified())
            .await
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    } else {
        assert!(sync.sync(99, Some(1)).await.is_err());
    }
    assert_eq!(remote.0.lock().unwrap().creates, 1);
    assert_eq!(project_id(&temp), 20);
    {
        let mut state = remote.0.lock().unwrap();
        state.pause_create = None;
        state.lose_create_response = None;
        if kind == ChannelType::GuildCategory {
            state.channels.get_mut(&1001).unwrap().name = "unreconciled-created-category".into();
            state.channels.insert(40, channel(40, None, kind, "Codex"));
        }
    }
    if kind == ChannelType::GuildText {
        foreign_project(&remote);
    }
    let sync = if cold {
        reconstructed(&temp, &remote)
    } else {
        sync
    };
    let retry = sync.sync(99, Some(1)).await;
    assert_eq!(
        project_id(&temp),
        20,
        "unknown create must not adopt a matching foreign topic"
    );
    assert!(
        retry.is_err(),
        "unknown category/project cannot be recovered by matching metadata"
    );
    let state = remote.0.lock().unwrap();
    assert_eq!(state.creates, 1, "unknown creation must not be repeated");
    assert!(state.channels.contains_key(&1001));
    assert!(state.channels.contains_key(&40));
}

async fn confirmed_project(cold: bool) {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    let sync = Arc::new(sync);
    let db = Connection::open(temp.path().join("mirror.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_project_mapping BEFORE INSERT ON mirror_projects
        WHEN NEW.discord_channel_id=1001 BEGIN SELECT RAISE(ABORT,'injected project map failure'); END;").unwrap();
    {
        let mut state = remote.0.lock().unwrap();
        state.channels.remove(&20);
        state.channels.remove(&30);
    }
    assert!(sync.sync(99, Some(1)).await.is_err());
    assert_eq!(remote.0.lock().unwrap().creates, 1);
    assert_eq!(project_id(&temp), 20);
    db.execute_batch("DROP TRIGGER reject_project_mapping;")
        .unwrap();
    foreign_project(&remote);
    let sync = if cold {
        reconstructed(&temp, &remote)
    } else {
        sync
    };
    sync.sync(99, Some(1)).await.unwrap();
    assert_eq!(
        project_id(&temp),
        1001,
        "use the confirmed response ID, never the foreign topic"
    );
    assert_eq!(
        thread_channels(&temp.path().join("mirror.sqlite"), "thread-a").unwrap(),
        Some((1001, 1002))
    );
    assert_eq!(
        remote.0.lock().unwrap().creates,
        2,
        "one project plus its replacement thread"
    );
}

#[tokio::test]
async fn mirror_container_project_lost_same() {
    unknown(ChannelType::GuildText, false, false).await;
}
#[tokio::test]
async fn mirror_container_project_lost_cold() {
    unknown(ChannelType::GuildText, false, true).await;
}
#[tokio::test]
async fn mirror_container_project_cancel_same() {
    unknown(ChannelType::GuildText, true, false).await;
}
#[tokio::test]
async fn mirror_container_project_cancel_cold() {
    unknown(ChannelType::GuildText, true, true).await;
}
#[tokio::test]
async fn mirror_container_category_lost_same() {
    unknown(ChannelType::GuildCategory, false, false).await;
}
#[tokio::test]
async fn mirror_container_category_lost_cold() {
    unknown(ChannelType::GuildCategory, false, true).await;
}
#[tokio::test]
async fn mirror_container_category_cancel_same() {
    unknown(ChannelType::GuildCategory, true, false).await;
}
#[tokio::test]
async fn mirror_container_category_cancel_cold() {
    unknown(ChannelType::GuildCategory, true, true).await;
}
#[tokio::test]
async fn mirror_container_project_confirmed_same() {
    confirmed_project(false).await;
}
#[tokio::test]
async fn mirror_container_project_confirmed_cold() {
    confirmed_project(true).await;
}

#[tokio::test]
async fn mirror_container_category_binding_failure_reuses_exact_id() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    let db = Connection::open(temp.path().join("mirror.sqlite")).unwrap();
    db.execute_batch(
        "CREATE TRIGGER fail_binding BEFORE UPDATE ON cdr_mirror_container_creations
        WHEN NEW.phase='bound' BEGIN SELECT RAISE(ABORT,'injected category binding failure'); END;",
    )
    .unwrap();
    remote.0.lock().unwrap().channels.remove(&10);
    assert!(sync.sync(99, Some(1)).await.is_err());
    assert_eq!(remote.0.lock().unwrap().creates, 1);
    db.execute_batch("DROP TRIGGER fail_binding;").unwrap();
    remote
        .0
        .lock()
        .unwrap()
        .channels
        .insert(40, channel(40, None, ChannelType::GuildCategory, "Codex"));
    reconstructed(&temp, &remote)
        .sync(99, Some(1))
        .await
        .unwrap();
    assert_eq!(remote.0.lock().unwrap().creates, 1);
    let bound = cdr_store::mapping::container_creation::category_receipt(
        &temp.path().join("mirror.sqlite"),
        1,
    )
    .unwrap()
    .unwrap();
    assert!(bound.is_bound());
    assert_eq!(bound.confirmed_channel().unwrap(), 1001);
}

#[tokio::test]
async fn mirror_container_bound_category_fresh_404_authorizes_one_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    remote.0.lock().unwrap().channels.remove(&10);
    sync.sync(99, Some(1)).await.unwrap();
    {
        let mut state = remote.0.lock().unwrap();
        state.channels.remove(&1001);
        state
            .channels
            .insert(40, channel(40, None, ChannelType::GuildCategory, "Codex"));
    }
    reconstructed(&temp, &remote)
        .sync(99, Some(1))
        .await
        .unwrap();
    assert_eq!(remote.0.lock().unwrap().creates, 2);
    let bound = cdr_store::mapping::container_creation::category_receipt(
        &temp.path().join("mirror.sqlite"),
        1,
    )
    .unwrap()
    .unwrap();
    assert_eq!(bound.confirmed_channel().unwrap(), 1002);
}

async fn intent_store_failure(kind: ChannelType) {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    Connection::open(temp.path().join("mirror.sqlite"))
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_intent BEFORE INSERT ON cdr_mirror_container_creations
         BEGIN SELECT RAISE(ABORT,'injected intent store failure'); END;",
        )
        .unwrap();
    remote
        .0
        .lock()
        .unwrap()
        .channels
        .remove(&if kind == ChannelType::GuildCategory {
            10
        } else {
            20
        });
    assert!(sync.sync(99, Some(1)).await.is_err());
    assert_eq!(remote.0.lock().unwrap().creates, 0);
    assert_eq!(project_id(&temp), 20);
}

#[tokio::test]
async fn mirror_container_category_intent_failure_sends_no_create() {
    intent_store_failure(ChannelType::GuildCategory).await;
}

#[tokio::test]
async fn mirror_container_project_intent_failure_sends_no_create() {
    intent_store_failure(ChannelType::GuildText).await;
}

#[tokio::test]
async fn mirror_container_claim_post_insert_mapping_change_sends_no_create() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    let path = temp.path().join("mirror.sqlite");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER change_project_during_claim
        AFTER INSERT ON cdr_mirror_container_creations WHEN NEW.kind='project'
        BEGIN UPDATE mirror_projects SET discord_channel_id=22; END;",
    )
    .unwrap();
    remote.0.lock().unwrap().channels.remove(&20);
    assert!(sync.sync(99, Some(1)).await.is_err());
    assert_eq!(
        remote.0.lock().unwrap().creates,
        0,
        "changed mapping must block the original POST"
    );
    assert_eq!(project_id(&temp), 20);
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM cdr_mirror_container_creations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    db.execute_batch("DROP TRIGGER change_project_during_claim;")
        .unwrap();
    remote.0.lock().unwrap().channels.remove(&30);
    sync.sync(99, Some(1)).await.unwrap();
    assert_eq!(project_id(&temp), 1001);
    assert_eq!(
        remote.0.lock().unwrap().creates,
        2,
        "healthy retry creates project and replacement thread once"
    );
}
