use super::*;
use crate::{
    message_worker,
    test_support::{app_fixture, http_gate, message_fixture::MessageFixture},
};
use cdr_store::{delivery, ingress};

#[derive(Clone, Copy)]
enum Output {
    Final,
    Commentary,
    Goal,
}

fn worker(fixture: &MessageFixture) -> CompletionWorker {
    CompletionWorker {
        server: fixture.server.clone(),
        queue: fixture.queue.clone(),
        http: fixture.http.clone(),
        commentary_enabled: true,
        history_read_timeout: Duration::from_secs(2),
        commentary: Mutex::new(CommentaryBuffer::default()),
        terminal_fence: terminal_fence::TerminalFence::default(),
    }
}

async fn early_output(
    worker: &CompletionWorker,
    output: Output,
) -> Result<Result<(), CompletionWorkerError>, tokio::time::error::Elapsed> {
    match output {
        Output::Commentary => {
            tokio::time::timeout(
                Duration::from_millis(200),
                worker.send_commentary(&CommentaryBlock {
                    thread_id: "thread-b".into(),
                    turn_id: "existing-turn".into(),
                    text: "checking".into(),
                }),
            )
            .await
        }
        Output::Final => {
            let pending = worker
                .queue
                .stage_turn_completion("thread-b", "existing-turn", "Final\nanswer")
                .await
                .unwrap()
                .unwrap();
            tokio::time::timeout(Duration::from_millis(200), worker.deliver_one(&pending)).await
        }
        Output::Goal => {
            let pending = worker
                .queue
                .stage_goal_progress("thread-b", "existing-turn", "[Goal progress]\nchecking")
                .await
                .unwrap()
                .unwrap();
            tokio::time::timeout(
                Duration::from_millis(200),
                worker.deliver_goal_progress(&pending),
            )
            .await
        }
    }
}

async fn verify_order(output: Output, lose_echo: bool) {
    let temp = tempfile::tempdir().unwrap();
    let mut gate = http_gate::start().await;
    let mut release = Some(gate.release);
    let http = Arc::new(
        Client::builder()
            .token("test-token".into())
            .proxy(gate.address, true)
            .ratelimiter(None)
            .timeout(Duration::from_secs(5))
            .build(),
    );
    let fixture = MessageFixture::new(&temp, http).await;
    let db = fixture.executor.mirror_db();
    let context = fixture.context(temp.path());
    let mut first_reply = Box::pin(message_worker::process_admitted_gateway_message(
        fixture.admit("input"),
        &context,
    ));
    tokio::select! {
        result = &mut first_reply => panic!("reply did not reach the HTTP gate: {result:?}"),
        entered = &mut gate.entered => entered.unwrap(),
    }
    let owner = ingress::get(db, "message:801").unwrap().unwrap();
    assert!(!owner.confirmation_delivered);
    assert_eq!(
        cdr_store::queue::list(db).unwrap()[0].turn_id.as_deref(),
        Some("existing-turn")
    );
    let original = worker(&fixture);
    let early = early_output(&original, output).await;
    let pending_before_echo = pending_count(db);
    if lose_echo {
        drop(first_reply);
    } else {
        release.take().unwrap().send(()).unwrap();
        first_reply.await.unwrap();
    }
    // No process-memory flag may be required for the ordering barrier.
    drop(original);
    let restarted = worker(&fixture);
    let after = restarted
        .deliver_pending()
        .await
        .and(restarted.recover_goal_progress().await);
    if lose_echo {
        release.take().unwrap().send(()).unwrap();
    }
    fixture.server.close().await.unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    assert!(
        early.is_ok(),
        "output tried HTTP before first echo confirmation"
    );
    assert!(
        early.unwrap().is_err(),
        "pending echo must be an explicit delivery hold"
    );
    assert_eq!(
        pending_before_echo, 1,
        "output must survive the ordering wait"
    );
    if lose_echo {
        assert!(after.is_err());
        assert_eq!(posts.len(), 1);
        assert_eq!(pending_count(db), 1);
        assert!(
            !ingress::get(db, "message:801")
                .unwrap()
                .unwrap()
                .confirmation_delivered
        );
    } else {
        after.unwrap();
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0]["content"], "In progress\nmessage: input");
        assert_eq!(
            posts[1]["content"],
            match output {
                Output::Commentary => "In progress\nchecking",
                Output::Final => "Final\nanswer",
                Output::Goal => "[Goal progress]\nchecking",
            }
        );
        assert_eq!(pending_count(db), 0);
    }
    assert_eq!(
        app_fixture::rpc_log(&temp.path().join("rpc.jsonl"))
            .iter()
            .filter(|rpc| rpc["method"] == "turn/start")
            .count(),
        1
    );
}

fn pending_count(db: &std::path::Path) -> usize {
    delivery::list_pending(db).unwrap().len()
        + cdr_store::commentary_outbox::pending(db).unwrap().len()
        + cdr_store::goal_progress::pending(db).unwrap().len()
}

#[tokio::test]
async fn final_waits_for_actual_message_worker_first_reply() {
    verify_order(Output::Final, false).await;
}
#[tokio::test]
async fn commentary_waits_for_actual_message_worker_first_reply() {
    verify_order(Output::Commentary, false).await;
}
#[tokio::test]
async fn uncertain_first_reply_keeps_final_held_after_reconstruction() {
    verify_order(Output::Final, true).await;
}
#[tokio::test]
async fn goal_progress_waits_for_actual_message_worker_first_reply() {
    verify_order(Output::Goal, false).await;
}

#[tokio::test]
async fn confirmed_echo_wakes_actual_processor_without_waiting_for_thirty_second_retry() {
    let temp = tempfile::tempdir().unwrap();
    let mut gate = http_gate::start().await;
    let fixture = MessageFixture::new(
        &temp,
        Arc::new(
            Client::builder()
                .token("test-token".into())
                .proxy(gate.address, true)
                .ratelimiter(None)
                .build(),
        ),
    )
    .await;
    let context = fixture.context(temp.path());
    let mut first_reply = Box::pin(message_worker::process_admitted_gateway_message(
        fixture.admit("input"),
        &context,
    ));
    tokio::select! {
        result = &mut first_reply => panic!("first reply bypassed gate: {result:?}"),
        entered = &mut gate.entered => entered.unwrap(),
    }
    let worker = Arc::new(worker(&fixture));
    early_output(&worker, Output::Final)
        .await
        .unwrap()
        .unwrap_err();
    let attempts_before =
        delivery::list_pending(fixture.executor.mirror_db()).unwrap()[0].attempt_count;
    let (events, pending) = tokio::sync::mpsc::channel(4);
    let processing = worker.clone();
    let task = tokio::spawn(async move { super::driver::process(&processing, pending).await });
    // The startup recovery has already attempted and held this exact outbox.
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let pending = delivery::list_pending(fixture.executor.mirror_db()).unwrap();
            if pending[0].attempt_count > attempts_before {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    gate.release.send(()).unwrap();
    first_reply.await.unwrap();
    let promptly_delivered = tokio::time::timeout(Duration::from_secs(2), async {
        while pending_count(fixture.executor.mirror_db()) != 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    drop(events);
    task.await.unwrap();
    fixture.server.close().await.unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    assert!(
        promptly_delivered.is_ok(),
        "confirmed first reply did not wake actual outbox processor"
    );
    assert_eq!(posts.len(), 2);
}
