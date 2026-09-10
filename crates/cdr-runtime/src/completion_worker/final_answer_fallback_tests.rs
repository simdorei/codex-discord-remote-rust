use super::goal_handoff_tests::{make_worker, setup_running};
use super::*;
use serde_json::json;

fn event(
    worker: &CompletionWorker,
    method: &str,
    params: serde_json::Value,
) -> ResidentNotificationEvent {
    ResidentNotificationEvent::Notification {
        generation: worker.server.generation(),
        notification: cdr_app_server::Notification {
            method: method.into(),
            params,
        },
    }
}

fn omit_history_and_complete_goal(temp: &tempfile::TempDir) {
    std::fs::write(temp.path().join("goal-rollout.jsonl.omit-history"), "1").unwrap();
    std::fs::write(temp.path().join("goal-rollout.jsonl.goal-complete"), "1").unwrap();
}

#[tokio::test]
async fn completed_notification_uses_durable_exact_final_when_history_omits_turn() {
    let temp = tempfile::tempdir().unwrap();
    omit_history_and_complete_goal(&temp);
    let worker = make_worker(&temp).await;
    setup_running(&worker);
    let item = event(
        &worker,
        "item/completed",
        json!({"threadId":"thread","turnId":"T1","item":{
            "id":"item","type":"agentMessage","phase":"final_answer","text":"exact fallback"
        }}),
    );
    worker.observe_terminal(&item).unwrap();
    worker.handle(item).await.unwrap();
    let terminal = event(
        &worker,
        "turn/completed",
        json!({"threadId":"thread","turn":{"id":"T1","status":"completed"}}),
    );
    worker.observe_terminal(&terminal).unwrap();
    let _ = worker.handle(terminal.clone()).await;
    let _ = worker.handle(terminal).await;

    assert!(
        cdr_store::queue::list(worker.queue.db_path())
            .unwrap()
            .is_empty()
    );
    let pending = cdr_store::delivery::list_pending(worker.queue.db_path()).unwrap();
    assert_eq!(pending.len(), 1);
    assert!(pending[0].content.contains("exact fallback"));
    assert_eq!(
        cdr_store::observed_final_answer::get(
            worker.queue.db_path(),
            "thread",
            "T1",
            i64::try_from(worker.server.generation()).unwrap(),
        )
        .unwrap(),
        None,
        "completion staging consumes the fallback in the same transaction"
    );
    worker.server.close().await.unwrap();
}

#[tokio::test]
async fn exact_final_and_terminal_survive_worker_restart_before_completion_processing() {
    let temp = tempfile::tempdir().unwrap();
    omit_history_and_complete_goal(&temp);
    let worker = make_worker(&temp).await;
    setup_running(&worker);
    let item = event(
        &worker,
        "item/completed",
        json!({"threadId":"thread","turnId":"T1","item":{
            "id":"item","type":"agentMessage","phase":"final_answer","text":"restart fallback"
        }}),
    );
    worker.observe_terminal(&item).unwrap();
    worker.server.close().await.unwrap();

    let restarted = make_worker(&temp).await;
    let _ = restarted.recover_observed().await;
    assert!(
        cdr_store::queue::list(restarted.queue.db_path())
            .unwrap()
            .is_empty()
    );
    let pending = cdr_store::delivery::list_pending(restarted.queue.db_path()).unwrap();
    assert_eq!(pending.len(), 1);
    assert!(pending[0].content.contains("restart fallback"));
    restarted.server.close().await.unwrap();
}

#[tokio::test]
async fn missing_history_and_missing_exact_final_unlocks_with_visible_recovery_error() {
    let temp = tempfile::tempdir().unwrap();
    omit_history_and_complete_goal(&temp);
    let worker = make_worker(&temp).await;
    setup_running(&worker);
    let terminal = event(
        &worker,
        "turn/completed",
        json!({"threadId":"thread","turn":{"id":"T1","status":"completed"}}),
    );
    let _ = worker.handle(terminal).await;

    assert!(
        cdr_store::queue::list(worker.queue.db_path())
            .unwrap()
            .is_empty()
    );
    let pending = cdr_store::delivery::list_pending(worker.queue.db_path()).unwrap();
    assert_eq!(pending.len(), 1);
    assert!(
        pending[0]
            .content
            .contains("exact final reply could not be recovered")
    );
    let methods = std::fs::read_to_string(temp.path().join("goal-rpc.log")).unwrap();
    assert_eq!(
        methods
            .lines()
            .filter(|method| *method == "thread/read")
            .count(),
        3,
        "history recovery must be brief and bounded"
    );
    worker.server.close().await.unwrap();
}
