//! Revision 12: real history recovery must inherit Goal ownership without replay.
#[path = "goal_inheritance_guard_tests.rs"]
mod goal_inheritance_guard_tests;
#[path = "goal_inheritance_regression_tests.rs"]
mod goal_inheritance_regression_tests;
#[path = "revision14_tests.rs"]
mod revision14_tests;
use super::*;
use cdr_store::goal_progress;

async fn completed_goal(f: &Fixture) -> String {
    f.configure(json!({"ordinary":true,"goal":"active"})).await;
    let turn = f.submit("goal-original").await.turn_id.unwrap();
    f.rpc(
        "test/complete",
        json!({"turnId":turn,"status":"completed","text":"inherited progress"}),
    )
    .await;
    turn
}

fn original(f: &Fixture) -> queue::StoredQueueJob {
    queue::list(&f.db)
        .unwrap()
        .into_iter()
        .find(|job| job.job_id == "goal-original")
        .unwrap()
}

fn make_legacy(f: &Fixture, legacy: bool) {
    if legacy {
        rusqlite::Connection::open(&f.db)
            .unwrap()
            .execute(
                "UPDATE codex_turn_queue SET execution_generation=NULL WHERE job_id='goal-original'",
                [],
            )
            .unwrap();
    }
}

fn assert_execution_preserved(before: &queue::StoredQueueJob, after: &queue::StoredQueueJob) {
    assert_eq!(after.job_id, before.job_id);
    assert_eq!(after.target_thread_id, before.target_thread_id);
    assert_eq!(after.channel_id, before.channel_id);
    assert_eq!(after.owner_user_id, before.owner_user_id);
    assert_eq!(after.prompt, before.prompt);
    assert_eq!(after.app_server_generation, before.app_server_generation);
    assert_eq!(after.execution_generation, before.execution_generation);
    assert_eq!(after.attempt_count, before.attempt_count);
    assert_eq!(after.baseline_turn_ids, before.baseline_turn_ids);
}

fn count_text(messages: &[Value], text: &str) -> usize {
    messages.iter().filter(|m| m["content"] == text).count()
}

#[tokio::test]
async fn inherited_active_goal_recovers_exact_progress_without_replaying_original() {
    for (new_runtime, legacy) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut f = Fixture::new().await;
        let turn = completed_goal(&f).await;
        make_legacy(&f, legacy);
        let before = original(&f);
        f.restart(new_runtime).await;
        assert_eq!(
            before.app_server_generation == i64::try_from(f.server.generation()).unwrap(),
            new_runtime
        );
        f.configure(json!({"ordinary":true,"goal":"active"})).await;
        let worker = f.worker();
        // No old completion boolean is injected: a new child's thread/read and
        // thread/goal/get drive the production queue + CompletionWorker recovery.
        worker.recover().await.unwrap();
        worker.recover().await.unwrap();
        let after = original(&f);
        assert!(after.goal_waiting);
        assert_eq!(after.turn_id.as_deref(), Some(turn.as_str()));
        assert_execution_preserved(&before, &after);
        assert!(goal_progress::pending(&f.db).unwrap().is_empty());
        assert_eq!(f.count("turn/start"), 1);
        assert_eq!(f.count("thread/settings/update"), 0);
        let messages = f.close().await;
        assert_eq!(
            count_text(&messages, "[Goal progress]\ninherited progress"),
            1
        );
    }
}

#[tokio::test]
async fn inherited_waiting_goal_attaches_observed_next_turn_and_delivers_final_once() {
    for (new_runtime, legacy) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut f = Fixture::new().await;
        let first = completed_goal(&f).await;
        f.worker().recover().await.unwrap();
        assert!(original(&f).goal_waiting);
        make_legacy(&f, legacy);
        let before = original(&f);
        f.restart(new_runtime).await;
        f.configure(json!({"ordinary":true,"goal":"active"})).await;
        // The fixture itself produces the automatic Goal turn. This is not a
        // bot turn/start and the bot must never resubmit the original prompt.
        let next = f.rpc("test/goal-turn", json!({})).await["turn"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_ne!(next, first);
        let worker = f.worker();
        worker.recover().await.unwrap();
        worker.recover().await.unwrap();
        let after = original(&f);
        assert_eq!(after.turn_id.as_deref(), Some(next.as_str()));
        assert!(!after.goal_waiting);
        assert_execution_preserved(&before, &after);
        f.rpc(
            "test/complete",
            json!({"turnId":next,"status":"completed","text":"inherited final"}),
        )
        .await;
        f.configure(json!({"goal":"complete"})).await;
        worker.recover().await.unwrap();
        worker.recover().await.unwrap();
        assert!(queue::list(&f.db).unwrap().is_empty());
        assert!(delivery::list_pending(&f.db).unwrap().is_empty());
        assert_eq!(f.count("turn/start"), 1);
        assert_eq!(f.count("thread/settings/update"), 0);
        let messages = f.close().await;
        assert_eq!(
            count_text(&messages, "[Goal progress]\ninherited progress"),
            1
        );
        assert_eq!(
            messages
                .iter()
                .filter(|m| m["content"]
                    .as_str()
                    .is_some_and(|s| s.contains("inherited final")))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn inherited_goal_continues_uses_the_stored_job_generation() {
    let mut f = Fixture::new().await;
    let turn = completed_goal(&f).await;
    let before = original(&f);
    f.restart(false).await;
    assert!(f.queue.goal_continues("thread-b", &turn).await.unwrap());
    assert!(original(&f).goal_waiting);
    assert_execution_preserved(&before, &original(&f));
    assert_eq!(f.count("turn/start"), 1);
    f.close().await;
}

#[tokio::test]
async fn inherited_goal_rejects_foreign_turn_and_preserves_the_original() {
    let mut f = Fixture::new().await;
    completed_goal(&f).await;
    f.restart(false).await;
    let before = original(&f);
    assert!(
        f.queue
            .stage_goal_progress("thread-b", "foreign-turn", "wrong")
            .await
            .is_err()
    );
    assert!(
        !f.queue
            .goal_continues("thread-b", "foreign-turn")
            .await
            .unwrap()
    );
    assert_eq!(original(&f), before);
    assert!(goal_progress::pending(&f.db).unwrap().is_empty());
    assert_eq!(f.count("turn/start"), 1);
    f.close().await;
}

#[tokio::test]
async fn inherited_goal_cannot_cross_a_dead_generation_hold() {
    let mut f = Fixture::new().await;
    let turn = completed_goal(&f).await;
    f.worker().recover().await.unwrap();
    cdr_store::dead_generation::activate_runtime(&f.db, "goal-runtime").unwrap();
    cdr_store::dead_generation::capture_dead_generation(
        &f.db,
        cdr_store::dead_generation::DeadGenerationCapture {
            runtime_id: "goal-runtime",
            generation: 1,
            snapshot_json: "{}",
            affected_targets: &["thread-b".into()],
            startup_channel_id: Some(42),
            has_unscoped_requests: false,
            now: 3.0,
        },
    )
    .unwrap();
    f.restart(false).await;
    let before = original(&f);
    assert!(
        f.queue
            .stage_goal_progress("thread-b", &turn, "held")
            .await
            .is_err()
    );
    assert!(!f.queue.goal_continues("thread-b", &turn).await.unwrap());
    assert!(!f.queue.goal_turn_started("thread-b", "next").await.unwrap());
    assert_eq!(original(&f), before);
    assert!(goal_progress::pending(&f.db).unwrap().is_empty());
    assert_eq!(f.count("turn/start"), 1);
    f.close().await;
}

#[tokio::test]
async fn inherited_waiting_owner_ignores_old_notification_but_accepts_current_observation() {
    let mut f = Fixture::new().await;
    completed_goal(&f).await;
    f.worker().recover().await.unwrap();
    let before = original(&f);
    let old_generation = f.server.generation();
    f.restart(false).await;
    f.configure(json!({"ordinary":true,"goal":"active"})).await;
    let next = f.rpc("test/goal-turn", json!({})).await["turn"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let event = |generation| ResidentNotificationEvent::Notification {
        generation,
        notification: cdr_app_server::Notification {
            method: "turn/started".into(),
            params: json!({"threadId":"thread-b","turn":{"id":next,"status":"inProgress"}}),
        },
    };
    let worker = f.worker();
    worker.handle(event(old_generation)).await.unwrap();
    assert_eq!(original(&f), before);
    worker.handle(event(f.server.generation())).await.unwrap();
    assert_eq!(original(&f).turn_id.as_deref(), Some(next.as_str()));
    assert_execution_preserved(&before, &original(&f));
    let attached = original(&f);
    worker.handle(event(f.server.generation())).await.unwrap();
    assert_eq!(original(&f), attached);
    assert_eq!(f.count("turn/start"), 1);
    f.close().await;
}

#[tokio::test]
async fn recovery_observation_cannot_rebind_a_replaced_waiting_owner() {
    let mut f = Fixture::new().await;
    completed_goal(&f).await;
    f.worker().recover().await.unwrap();
    let captured_before_observation = original(&f);
    f.restart(false).await;
    queue::complete(&f.db, "goal-original").unwrap();
    queue::enqueue(
        &f.db,
        queue::NewQueueJob {
            job_id: "replacement",
            target_thread_id: "thread-b",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: None,
            app_server_generation: 1,
            prompt: "different job",
            queued: false,
            ack_sent: true,
            created_at: 5.0,
        },
    )
    .unwrap();
    queue::begin_attempt(&f.db, "replacement", &[], 1).unwrap();
    queue::mark_running(&f.db, "replacement", "replacement-turn", 1).unwrap();
    queue::mark_goal_waiting(&f.db, "replacement", "replacement-turn", 1).unwrap();
    let before = queue::list(&f.db).unwrap();
    assert!(
        !f.queue
            .goal_turn_started_observed(
                "thread-b",
                "observed-next",
                f.server.generation(),
                Some(&captured_before_observation),
            )
            .await
            .unwrap()
    );
    assert_eq!(queue::list(&f.db).unwrap(), before);
    assert_eq!(f.count("turn/start"), 1);
    f.close().await;
}

#[tokio::test]
async fn inherited_progress_receipt_hold_preserves_handoff_without_http_repost() {
    use sha2::{Digest, Sha256};
    for blocked in [false, true] {
        let mut f = Fixture::new().await;
        let turn = completed_goal(&f).await;
        let identity = format!("{}:thread-b;{}:{turn};", "thread-b".len(), turn.len());
        let key =
            serde_json::to_string(&(42_u64, "completion/goal-progress/v1", identity, 0)).unwrap();
        let hash = hex::encode(Sha256::digest(b"[Goal progress]\ninherited progress"));
        cdr_store::delivery_receipt::begin(&f.db, &key, &hash).unwrap();
        if blocked {
            cdr_store::delivery_receipt::block_rejected(&f.db, &key, "403 test rejection").unwrap();
        }
        f.restart(false).await;
        f.configure(json!({"ordinary":true,"goal":"active"})).await;
        let worker = f.worker();
        let error = worker.recover().await.unwrap_err().to_string();
        assert!(
            error.contains(if blocked {
                "requires correction"
            } else {
                "outcome unknown"
            }),
            "{error}"
        );
        assert!(original(&f).goal_waiting);
        let pending = goal_progress::pending(&f.db).unwrap();
        assert_eq!(pending.len(), 1);
        assert!(!pending[0].last_error.is_empty());
        let next = f.rpc("test/goal-turn", json!({})).await["turn"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        worker.recover().await.unwrap();
        assert_eq!(original(&f).turn_id.as_deref(), Some(next.as_str()));
        for _ in 0..2 {
            assert!(worker.recover_goal_progress().await.is_err());
        }
        assert_eq!(goal_progress::pending(&f.db).unwrap().len(), 1);
        assert_eq!(f.count("turn/start"), 1);
        assert!(
            f.close().await.is_empty(),
            "held progress must not be sent again"
        );
    }
}
