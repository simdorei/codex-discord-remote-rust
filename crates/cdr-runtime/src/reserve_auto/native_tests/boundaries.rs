use super::*;
use crate::restart_readiness::drain::DrainFenceKey;
use tokio::sync::watch;

async fn wait_gate(f: &Fixture) {
    tokio::time::timeout(Duration::from_secs(4), async {
        while !f.temp.path().join("gate-ready").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("native fixture never reached the configured barrier");
}
fn release_gate(f: &Fixture) {
    std::fs::write(f.temp.path().join("gate-release"), "release").unwrap();
}

#[tokio::test]
async fn recovery_and_manual_change_share_the_same_target_lock() {
    let f = Fixture::new().await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    f.configure(json!({"ordinary":true,"gate":"thread/settings/update"}))
        .await;
    let q = f.queue.clone();
    let c = f.controller.clone();
    let recovery = tokio::spawn(async move { q.recover_reserve_target(&c, "thread-b").await });
    wait_gate(&f).await;
    let executor = f.executor.clone();
    let manual = tokio::spawn(async move {
        executor
            .execute(
                CommandAction::Settings {
                    reference: Some("thread-b".into()),
                    model: Some("model-b".into()),
                    effort: Some("medium".into()),
                    speed: None,
                },
                42,
                3,
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(
        !manual.is_finished(),
        "manual mutation bypassed the recovery target lock"
    );
    release_gate(&f);
    recovery.await.unwrap().unwrap();
    manual.await.unwrap().unwrap();
    f.queue
        .recover_reserve_target(&f.controller, "thread-b")
        .await
        .unwrap();
    assert_eq!(f.rpc("thread/resume", json!({})).await["model"], "model-b");
    assert_eq!(
        reserve_policy::get(&f.db, "thread-b")
            .unwrap()
            .unwrap()
            .mode,
        "manual"
    );
    assert_eq!(f.count("thread/settings/update"), 3);
    f.close().await;
}

#[tokio::test]
async fn recovery_and_new_submission_are_serialized_without_duplicate_start() {
    let f = Fixture::new().await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    f.configure(json!({"ordinary":true,"gate":"thread/settings/update"}))
        .await;
    let q = f.queue.clone();
    let c = f.controller.clone();
    let recovery = tokio::spawn(async move { q.recover_reserve_target(&c, "thread-b").await });
    wait_gate(&f).await;
    let q = f.queue.clone();
    let submit = tokio::spawn(async move {
        q.submit_identified("after-restore", "thread-b", 42, 3, None, "new input")
            .await
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!submit.is_finished());
    assert_eq!(f.count("turn/start"), 0);
    release_gate(&f);
    recovery.await.unwrap().unwrap();
    let job = submit.await.unwrap().unwrap();
    assert!(job.turn_id.is_some());
    assert_eq!(f.count("turn/start"), 1);
    let start = f
        .frames()
        .into_iter()
        .find(|v| v["event"] == "start_settings")
        .unwrap();
    assert_eq!(start["settings"]["model"], "model-a");
    f.close().await;
}

#[tokio::test]
async fn drain_sealed_while_recovery_waits_for_the_lock_prevents_mutation() {
    let f = Fixture::new().await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    f.configure(json!({"ordinary":true})).await;
    let lock = f.queue.target_lock("thread-b").unwrap();
    let guard = lock.lock().await;
    let q = f.queue.clone();
    let c = f.controller.clone();
    let recovery = tokio::spawn(async move { q.recover_reserve_target(&c, "thread-b").await });
    tokio::task::yield_now().await;
    f.gate
        .seal(&DrainFenceKey::new("test-runtime", "1|2", "test-nonce").unwrap())
        .unwrap();
    drop(guard);
    let result = recovery.await.unwrap();
    assert!(
        result.is_ok()
            || matches!(
                result,
                Err(crate::queue_runner::QueueRunnerError::RestartDrain(_))
            )
    );
    assert_eq!(f.count("thread/settings/update"), 1);
    f.close().await;
}

#[tokio::test]
async fn active_or_preparing_jobs_prevent_background_restore() {
    let f = Fixture::new().await;
    let a = f.submit("active-job").await;
    f.configure(json!({"ordinary":true})).await;
    f.queue
        .recover_reserve_target(&f.controller, "thread-b")
        .await
        .unwrap();
    assert_eq!(f.count("thread/settings/update"), 1);
    f.finish(a.turn_id.as_deref().unwrap(), false).await;
    let generation = i64::try_from(f.server.generation()).unwrap();
    cdr_store::queue::enqueue(
        &f.db,
        cdr_store::queue::NewQueueJob {
            job_id: "preparing",
            target_thread_id: "thread-b",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: None,
            app_server_generation: generation,
            prompt: "pending input",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    cdr_store::queue::begin_attempt(&f.db, "preparing", &[], generation).unwrap();
    f.queue
        .recover_reserve_target(&f.controller, "thread-b")
        .await
        .unwrap();
    assert_eq!(f.count("thread/settings/update"), 1);
    f.close().await;
}

#[tokio::test]
async fn cycle_timeout_waiting_for_a_lock_does_not_quarantine_someone_elses_episode() {
    let f = Fixture::new().await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    let before = reserve_policy::get(&f.db, "thread-b").unwrap();
    let lock = f.queue.target_lock("thread-b").unwrap();
    let guard = lock.lock().await;
    let (_sender, mut shutdown) = watch::channel(false);
    let mut cursor = None;
    assert!(
        !super::super::run_recovery_cycle(
            &f.controller,
            &f.queue,
            &mut shutdown,
            &mut cursor,
            Duration::from_millis(30)
        )
        .await
    );
    assert_eq!(reserve_policy::get(&f.db, "thread-b").unwrap(), before);
    assert_eq!(cursor.as_deref(), Some("thread-b"));
    drop(guard);
    f.close().await;
}

#[tokio::test]
async fn shutdown_during_dispatched_restore_preserves_intent_and_never_retries() {
    let f = Fixture::new().await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    f.configure(json!({"ordinary":true,"gate":"thread/settings/update"}))
        .await;
    let (sender, mut shutdown) = watch::channel(false);
    let q = f.queue.clone();
    let c = f.controller.clone();
    let worker = tokio::spawn(async move {
        super::super::run_recovery_cycle(&c, &q, &mut shutdown, &mut None, Duration::from_secs(5))
            .await
    });
    wait_gate(&f).await;
    sender.send(true).unwrap();
    assert!(worker.await.unwrap());
    let policy = reserve_policy::get(&f.db, "thread-b").unwrap().unwrap();
    assert_eq!(policy.state, "restoring");
    assert_eq!(policy.previous_model.as_deref(), Some("model-a"));
    assert_eq!(f.count("thread/settings/update"), 2);
    release_gate(&f);
    f.controller.set_manual_mode("thread-b", false).unwrap();
    assert!(f.controller.prepare_turn("thread-b").await.is_err());
    assert_eq!(f.count("thread/settings/update"), 2);
    f.close().await;
}

#[tokio::test]
async fn store_commit_failure_after_server_apply_is_held_without_settings_replay() {
    let f = Fixture::new().await;
    reserve_policy::ensure(&f.db, "thread-b").unwrap();
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_auto_finish BEFORE UPDATE OF state ON codex_reserve_policy WHEN OLD.state='entering' AND NEW.state='reserve' BEGIN SELECT RAISE(ABORT,'injected final commit failure'); END;").unwrap();
    assert!(f.controller.prepare_turn("thread-b").await.is_err());
    assert_eq!(
        reserve_policy::get(&f.db, "thread-b")
            .unwrap()
            .unwrap()
            .state,
        "unknown"
    );
    conn.execute_batch("DROP TRIGGER reject_auto_finish;")
        .unwrap();
    drop(conn);
    assert!(f.controller.prepare_turn("thread-b").await.is_err());
    assert_eq!(f.count("thread/settings/update"), 1);
    f.close().await;
}

#[tokio::test]
async fn restart_with_changed_live_settings_and_missing_identity_still_fail_closed() {
    let f = Fixture::new().await;
    assert!(
        f.controller
            .require_identity((None, i64::try_from(f.server.generation()).unwrap()))
            .await
            .is_err()
    );
    f.controller.prepare_turn("thread-b").await.unwrap();
    assert!(f.server.force_restart_if_quiescent().await.unwrap());
    assert!(f.controller.prepare_turn("thread-b").await.is_err());
    assert_eq!(
        reserve_policy::get(&f.db, "thread-b")
            .unwrap()
            .unwrap()
            .state,
        "unknown"
    );
    assert_eq!(f.count("thread/settings/update"), 1);
    f.close().await;
}

#[tokio::test]
async fn on_off_and_manual_noop_preserve_or_cancel_the_correct_owner() {
    let f = Fixture::new().await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    f.controller.set_manual_mode("thread-b", true).unwrap();
    let on = reserve_policy::get(&f.db, "thread-b").unwrap().unwrap();
    assert_eq!(on.state, "reserve");
    assert_eq!(on.previous_model.as_deref(), Some("model-a"));
    f.controller.set_manual_mode("thread-b", false).unwrap();
    f.controller.set_manual_mode("thread-b", true).unwrap();
    let result = f
        .executor
        .execute(
            CommandAction::Settings {
                reference: Some("thread-b".into()),
                model: Some("reserve".into()),
                effort: Some("medium".into()),
                speed: None,
            },
            42,
            3,
        )
        .await
        .unwrap();
    assert!(result.text.contains("이미 적용된 설정"));
    assert_eq!(f.count("thread/settings/update"), 1);
    assert_eq!(
        reserve_policy::get(&f.db, "thread-b")
            .unwrap()
            .unwrap()
            .mode,
        "manual"
    );
    f.close().await;
}

#[test]
fn stale_cas_cannot_complete_or_quarantine_a_new_episode() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let a = reserve_policy::ensure(&db, "t").unwrap();
    let first = reserve_policy::begin_episode_claim(
        &db,
        "t",
        a.revision,
        reserve_policy::EpisodeIdentity {
            account_id: "account-a",
            process_id: Some(1),
            generation: 1,
        },
        ("a", Some("high"), None),
        true,
        (Some("gpt-reserve"), Some("high"), Some("default")),
    )
    .unwrap()
    .unwrap();
    reserve_policy::set_mode(&db, "t", "manual").unwrap();
    let b = reserve_policy::set_mode(&db, "t", "on").unwrap();
    let second = reserve_policy::begin_episode_claim(
        &db,
        "t",
        b.revision,
        reserve_policy::EpisodeIdentity {
            account_id: "account-a",
            process_id: Some(1),
            generation: 1,
        },
        ("b", Some("low"), None),
        true,
        (Some("gpt-reserve"), Some("low"), Some("default")),
    )
    .unwrap()
    .unwrap();
    assert!(!reserve_policy::finish_episode_claim(&db, "t", first, "entering", "reserve").unwrap());
    assert!(!reserve_policy::mark_unknown_claim(&db, "t", first.revision, "stale").unwrap());
    assert_eq!(
        reserve_policy::get(&db, "t").unwrap().unwrap().revision,
        second.revision
    );
    let off = reserve_policy::set_mode(&db, "t", "off").unwrap();
    assert_eq!(off.state, "entering");
    assert_eq!(off.previous_model.as_deref(), Some("b"));
}

#[test]
fn keyset_recovery_pages_reach_targets_after_busy_and_unknown_rows() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let conn = cdr_store::schema::open_initialized(&db).unwrap();
    for i in 0..130 {
        conn.execute(
            "INSERT INTO codex_reserve_policy(thread_id,mode,state) VALUES(?1,'auto','reserve')",
            [format!("target-{i:03}")],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO codex_reserve_policy(thread_id,mode,state) VALUES(?1,'auto','unknown')",
            [format!("old-{i:03}")],
        )
        .unwrap();
    }
    let first = reserve_policy::recovery_candidates_after(&db, None).unwrap();
    let second =
        reserve_policy::recovery_candidates_after(&db, first.last().map(String::as_str)).unwrap();
    assert_eq!(first.len(), 128);
    assert_eq!(second, vec!["target-128", "target-129"]);
    assert!(
        reserve_policy::recovery_candidates_after(&db, second.last().map(String::as_str))
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn normalized_terminal_journal_is_bounded_and_drives_the_real_usage_hook() {
    let f = Fixture::new().await;
    f.configure(json!({"ordinary":true})).await;
    let a = f.submit("journal-a").await;
    let turn = a.turn_id.as_deref().unwrap();
    let mut event = f
        .rpc("test/complete", json!({"turnId":turn,"status":"failed"}))
        .await;
    event["turn"]["error"]["additionalDetails"] = json!("private detail".repeat(1000));
    let completion = parse_turn_completion(&event, false).unwrap();
    let payload = cdr_app_server::outcomes::completion_journal_payload(&completion).to_string();
    assert!(payload.len() < 2000);
    assert!(!payload.contains("private detail"));
    let generation = i64::try_from(f.server.generation()).unwrap();
    assert!(
        cdr_store::observed_completion::record(&f.db, "thread-b", turn, generation, &payload)
            .unwrap()
    );
    let rows = cdr_store::observed_completion::pending_with_generation(&f.db).unwrap();
    let recovered =
        parse_turn_completion(&serde_json::from_str::<Value>(&rows[0].3).unwrap(), false).unwrap();
    f.queue
        .stage_turn_completion_with_usage_limit(
            "thread-b",
            turn,
            &recovered.error_message,
            recovered.usage_limit,
        )
        .await
        .unwrap();
    assert_eq!(
        reserve_policy::get(&f.db, "thread-b")
            .unwrap()
            .unwrap()
            .state,
        "reserve"
    );
    assert_eq!(f.count("turn/start"), 1);
    f.close().await;
}

#[tokio::test]
async fn queued_start_rejection_has_a_durable_visible_failure_not_only_a_log() {
    let f = Fixture::new().await;
    f.configure(json!({"ordinary":true})).await;
    let a = f.submit("before-rejection").await;
    let b = f.submit("queued-rejection").await;
    assert!(b.queued);
    f.configure(json!({"reject_next_start":"usage"})).await;
    let turn = a.turn_id.as_deref().unwrap();
    f.rpc("test/complete", json!({"turnId":turn,"status":"completed"}))
        .await;
    assert!(
        f.queue
            .stage_turn_completion("thread-b", turn, "finished first")
            .await
            .is_err()
    );
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM codex_reserve_start_notices WHERE job_id='queued-rejection'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(
        count, 1,
        "queued no-turn rejection was not staged for durable delivery"
    );
    assert_eq!(f.count("turn/start"), 2);
    f.close().await;
}
