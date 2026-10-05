use super::*;

#[tokio::test]
async fn rr01_explicit_alias_keeps_its_original_uuid_after_sort_and_mapping_changes() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp, "ready").await;
    let _admitted = admit(&f, "!repair 1");
    let record = cdr_store::ingress::get(f.executor.mirror_db(), "message:801")
        .unwrap()
        .unwrap();
    assert_eq!(record.payload["lifecycle_binding"]["target"], "thread-a");
    rusqlite::Connection::open(temp.path().join("state.sqlite"))
        .unwrap()
        .execute("UPDATE threads SET updated_at=100 WHERE id='thread-b'", [])
        .unwrap();
    remap(&f);
    f.executor
        .execute_with_ingress_context(
            CommandAction::Repair {
                reference: Some("1".into()),
            },
            context(),
            "message:801",
        )
        .await
        .unwrap();
    let sent = calls(&temp);
    f.server.close().await.unwrap();
    assert_eq!(sent.len(), 6);
    assert!(
        sent.iter()
            .skip(1)
            .all(|c| c["params"]["threadId"] == "thread-a")
    );
}

#[tokio::test]
async fn rr01_selected_route_changes_and_ambiguous_references_never_dispatch() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp, "ready").await;
    cdr_store::mapping::upsert_thread(
        f.executor.mirror_db(),
        "thread-b",
        "project",
        "B",
        100,
        43,
        3.0,
    )
    .unwrap();
    f.executor
        .execute(
            CommandAction::Use {
                reference: "thread-a".into(),
            },
            42,
            3,
        )
        .await
        .unwrap();
    let _admitted = admit(&f, "!repair");
    let record = cdr_store::ingress::get(f.executor.mirror_db(), "message:801")
        .unwrap()
        .unwrap();
    assert_eq!(record.payload["lifecycle_binding"]["route"], "Selected");
    f.executor
        .execute(
            CommandAction::Use {
                reference: "thread-b".into(),
            },
            42,
            3,
        )
        .await
        .unwrap();
    assert!(
        f.executor
            .execute_with_ingress_context(
                CommandAction::Repair { reference: None },
                context(),
                "message:801"
            )
            .await
            .is_err()
    );
    let _ambiguous = f.admit_id("!repair thread", 802);
    let record = cdr_store::ingress::get(f.executor.mirror_db(), "message:802")
        .unwrap()
        .unwrap();
    assert!(record.payload["plan"]["Respond"].is_string());
    assert!(record.payload["lifecycle_binding"].is_null());
    let count = calls(&temp).len();
    f.server.close().await.unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn rr01_remap_while_waiting_for_target_lock_is_rechecked_after_acquisition() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp, "ready").await;
    let _admitted = admit(&f, "!repair");
    let lock = f.queue.target_lock("thread-b").unwrap().lock_owned().await;
    let command = f.executor.execute_with_ingress_context(
        CommandAction::Repair { reference: None },
        context(),
        "message:801",
    );
    let remapping = async {
        tokio::time::timeout(Duration::from_millis(800), async {
            loop {
                let record = cdr_store::ingress::get(f.executor.mirror_db(), "message:801")
                    .unwrap()
                    .unwrap();
                if record.phase == "recovery_claimed" {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        remap(&f);
        drop(lock);
    };
    let (result, ()) = tokio::join!(command, remapping);
    let count = calls(&temp).len();
    f.server.close().await.unwrap();
    assert!(result.is_err());
    assert_eq!(count, 1);
}

#[tokio::test]
async fn rr01_context_mismatch_and_route_tampering_do_not_acquire_effect_authority() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp, "ready").await;
    let _admitted = admit(&f, "!repair");
    for field in 0..3 {
        let mut wrong = context();
        match field {
            0 => wrong.user_id = 4,
            1 => wrong.channel_id = 43,
            _ => wrong.discord_message_id = Some(802),
        }
        assert!(
            f.executor
                .execute_with_ingress_context(
                    CommandAction::Repair { reference: None },
                    wrong,
                    "message:801"
                )
                .await
                .is_err()
        );
    }
    rusqlite::Connection::open(f.executor.mirror_db()).unwrap().execute(
        "UPDATE discord_ingress_journal SET payload_json=json_set(payload_json,'$.lifecycle_binding.route','Explicit') WHERE ingress_id='message:801'", []).unwrap();
    assert!(
        f.executor
            .execute_with_ingress_context(
                CommandAction::Repair { reference: None },
                context(),
                "message:801"
            )
            .await
            .is_err()
    );
    let count = calls(&temp).len();
    f.server.close().await.unwrap();
    assert_eq!(count, 1);
}

#[cfg(windows)]
#[tokio::test]
async fn rr01_recover_is_once_only_and_does_not_wait_on_the_stuck_target_lock() {
    let temp = tempfile::tempdir().unwrap();
    let mut f = fixture(&temp, "ready").await;
    fake_controller(&temp, &mut f);
    let _admitted = admit(&f, "!recover");
    enqueue(&f, "original", "thread-b", 501);
    enqueue(&f, "other", "thread-a", 502);
    let _lock = f.queue.target_lock("thread-b").unwrap().lock_owned().await;
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        f.executor.execute_with_ingress_context(
            CommandAction::Recover { reference: None },
            context(),
            "message:801",
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(result.text.contains("1개"));
    assert!(
        f.executor
            .execute_with_ingress_context(
                CommandAction::Recover { reference: None },
                context(),
                "message:801"
            )
            .await
            .is_err()
    );
    let remaining = cdr_store::queue::list(f.executor.mirror_db()).unwrap();
    let host = std::fs::read_to_string(temp.path().join("fake-controller/calls.txt")).unwrap();
    f.server.close().await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].job_id, "other");
    assert_eq!(
        host.lines().collect::<Vec<_>>(),
        ["InspectTools:thread-b", "StartTools:thread-b"]
    );
}

#[cfg(windows)]
#[tokio::test]
async fn rr01_cancellation_transaction_rolls_back_when_the_claim_changes_inside_it() {
    let temp = tempfile::tempdir().unwrap();
    let mut f = fixture(&temp, "ready").await;
    fake_controller(&temp, &mut f);
    let _admitted = admit(&f, "!recover");
    enqueue(&f, "original", "thread-b", 501);
    let before = cdr_store::queue::list(f.executor.mirror_db()).unwrap();
    rusqlite::Connection::open(f.executor.mirror_db())
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER change_recovery_owner AFTER DELETE ON codex_turn_queue BEGIN
         UPDATE discord_ingress_journal SET owner_user_id=99 WHERE ingress_id='message:801'; END;",
        )
        .unwrap();
    let result = f
        .executor
        .execute_with_ingress_context(
            CommandAction::Recover { reference: None },
            context(),
            "message:801",
        )
        .await;
    let after = cdr_store::queue::list(f.executor.mirror_db()).unwrap();
    let owner = cdr_store::ingress::get(f.executor.mirror_db(), "message:801")
        .unwrap()
        .unwrap()
        .owner_user_id;
    let host = std::fs::read_to_string(temp.path().join("fake-controller/calls.txt")).unwrap();
    f.server.close().await.unwrap();
    assert!(result.is_err());
    assert_eq!(after, before);
    assert_eq!(owner, 3);
    assert_eq!(host.lines().collect::<Vec<_>>(), ["InspectTools:thread-b"]);
}

#[cfg(windows)]
#[tokio::test]
async fn rr01_post_commit_remap_preserves_cancellation_but_refuses_restart() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let temp = tempfile::tempdir().unwrap();
    let mut f = fixture(&temp, "ready").await;
    fake_controller(&temp, &mut f);
    let _admitted = admit(&f, "!recover");
    enqueue(&f, "original", "thread-b", 501);
    let record = cdr_store::ingress::get(f.executor.mirror_db(), "message:801")
        .unwrap()
        .unwrap();
    let binding = serde_json::from_value(record.payload["lifecycle_binding"].clone()).unwrap();
    let guard = f
        .executor
        .claim_recovery_guard(42, binding, &record)
        .unwrap();
    let checks = AtomicUsize::new(0);
    let root = temp.path().join("fake-controller");
    let result = crate::writer_recovery::run_tools_checked(
        crate::writer_recovery::Target {
            root: &root,
            codex_home: temp.path(),
            database: f.executor.mirror_db(),
            thread: "thread-b",
            channel: 42,
            user: 3,
        },
        false,
        &|| {
            if checks.fetch_add(1, Ordering::SeqCst) == 1 {
                remap(&f);
            }
            guard.check().map_err(|e| e.to_string())
        },
        &|db| guard.check_in(db),
    )
    .await;
    let remaining = cdr_store::queue::list(f.executor.mirror_db()).unwrap();
    let host = std::fs::read_to_string(root.join("calls.txt")).unwrap();
    f.server.close().await.unwrap();
    assert!(
        result
            .unwrap_err()
            .contains("1 requests were cancelled and will not replay")
    );
    assert!(remaining.is_empty());
    assert_eq!(host.lines().collect::<Vec<_>>(), ["InspectTools:thread-b"]);
}
