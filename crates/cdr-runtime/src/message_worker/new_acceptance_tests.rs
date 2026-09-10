use crate::{
    action_executor::ActionContext,
    app_backend::AppServerTurnBackend,
    command_plan::CommandAction,
    queue_runner::QueueCoordinator,
    test_support::{app_fixture, new_reply_fixture},
};
use cdr_store::{
    new_reply,
    queue::{self, QueueJobState},
};
use serde_json::json;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

async fn recovery_mode(
    server: &cdr_app_server::ResidentAppServer,
    drop_response: bool,
    hide_history: bool,
) {
    server
        .execute(
            cdr_app_server::requests::AppRequest {
                method: "test/new-recovery-mode",
                params: json!({"drop_response":drop_response,"hide_history":hide_history}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
}

async fn uncertain_new_acceptance(lost_response: bool) {
    let temp = tempfile::tempdir().unwrap();
    let (fixture, remote, gate) = new_reply_fixture::setup(&temp).await;
    let db = fixture.executor.mirror_db();
    recovery_mode(&fixture.server, lost_response, lost_response).await;
    if !lost_response {
        cdr_store::schema::open_initialized(db).unwrap().execute_batch(
            "CREATE TRIGGER reject_first_binding BEFORE UPDATE OF turn_id ON codex_new_first_replies WHEN NEW.turn_id IS NOT NULL BEGIN SELECT RAISE(ABORT,'fixture exact binding commit failed'); END;"
        ).unwrap();
    }
    let admitted = fixture.admit("!new first");
    let mut request = Box::pin(fixture.executor.execute_with_context(
        CommandAction::New {
            prompt: "first".into(),
        },
        ActionContext {
            channel_id: 42,
            user_id: 3,
            discord_message_id: Some(801),
            auto_queue_when_busy: true,
        },
    ));
    if lost_response {
        tokio::time::timeout(Duration::from_secs(3),async {
            loop {
                tokio::select! {
                    value=&mut request => panic!("acceptance response unexpectedly arrived: {value:?}"),
                    ()=tokio::time::sleep(Duration::from_millis(10))=>{}
                }
                if app_fixture::rpc_log(&temp.path().join("rpc.jsonl")).iter().any(|r|r["method"]=="turn/start") {break;}
            }
        }).await.unwrap();
        drop(request); // response is lost after the peer durably accepted the turn
    } else {
        let error = request.await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("fixture exact binding commit failed")
        );
        cdr_store::schema::open_initialized(db)
            .unwrap()
            .execute_batch("DROP TRIGGER reject_first_binding;")
            .unwrap();
    }
    let before = queue::list(db).unwrap().remove(0);
    assert_eq!(before.state, QueueJobState::Starting);
    assert!(
        new_reply::get(db, &before.job_id)
            .unwrap()
            .unwrap()
            .turn_id
            .is_none()
    );
    // Advance the durable lease clock, then recreate coordinator/backend to
    // exercise cold startup without the old in-memory fresh-thread shortcut.
    cdr_store::schema::open_initialized(db)
        .unwrap()
        .execute(
            "UPDATE codex_turn_queue SET updated_at=0 WHERE job_id=?",
            [&before.job_id],
        )
        .unwrap();
    fixture.server.close().await.unwrap();
    let server =
        Arc::new(app_fixture::start_fake_server(&temp, &temp.path().join("rpc.jsonl")).await);
    recovery_mode(&server, false, lost_response).await;
    let restarted = QueueCoordinator::new(
        db.to_path_buf(),
        Arc::new(AppServerTurnBackend::new(server.clone())),
    );
    restarted.recover_target("new-thread").await.unwrap();
    let after_missing_history = queue::list(db).unwrap().remove(0);
    recovery_mode(&server, false, false).await;
    restarted.recover_target("new-thread").await.unwrap();
    restarted.recover_target("new-thread").await.unwrap();
    let after = queue::list(db).unwrap().remove(0);
    let intent = new_reply::get(db, &before.job_id).unwrap().unwrap();
    server.close().await.unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    let rpc = app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
    assert_single_execution(&rpc);
    assert_eq!(remote.creates.load(Ordering::SeqCst), 1);
    assert!(posts.is_empty());
    if lost_response {
        assert_eq!(
            after_missing_history.state,
            QueueJobState::Starting,
            "empty history is not evidence that an accepted new turn did not run"
        );
    }
    assert_eq!(after.state, QueueJobState::Running);
    assert_eq!(after.turn_id.as_deref(), Some("first-turn"));
    assert_eq!(intent.turn_id.as_deref(), Some("first-turn"));
    drop(admitted);
}

fn assert_single_execution(rpc: &[serde_json::Value]) {
    for (method, expected) in [("thread/start", 1), ("turn/start", 1), ("thread/fork", 0)] {
        assert_eq!(
            rpc.iter().filter(|r| r["method"] == method).count(),
            expected,
            "{method}"
        );
    }
}

#[tokio::test]
async fn lost_new_turn_response_and_empty_history_never_authorize_reexecution() {
    uncertain_new_acceptance(true).await;
}

#[tokio::test]
async fn failed_first_turn_binding_commit_recovers_same_turn_without_reexecution() {
    uncertain_new_acceptance(false).await;
}
