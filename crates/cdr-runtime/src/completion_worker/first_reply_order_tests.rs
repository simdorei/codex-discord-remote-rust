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
async fn explicit_saved_final_recovery_posts_once_without_reopening_acceptance_or_replaying() {
    use crate::test_support::approval_http;
    use cdr_store::{delivery_receipt, final_recovery};
    use serde_json::json;
    let temp = tempfile::tempdir().unwrap();
    let http = approval_http::start().await;
    let fixture = MessageFixture::new(
        &temp,
        Arc::new(
            Client::builder()
                .proxy(http.address.clone(), true)
                .ratelimiter(None)
                .build(),
        ),
    )
    .await;
    let db = fixture.queue.db_path();
    let result = fixture
        .queue
        .submit_identified("saved", "thread-b", 42, 3, Some(801), "input")
        .await
        .unwrap();
    rusqlite::Connection::open(db).unwrap().execute_batch("INSERT INTO discord_ingress_journal
        (ingress_id,kind,event_id,channel_id,owner_user_id,payload_json,state,phase,target_thread_id,
        owner_kind,owner_id,confirmation_delivered,created_at,updated_at)
        VALUES ('message:801','message',801,42,3,'{}','owned','durable_prompt','thread-b','prompt','saved',0,1,1)").unwrap();
    let key = json!([
        42,
        "message/error/v1",
        "inbound-message/801/error-report",
        0
    ])
    .to_string();
    delivery_receipt::begin(db, &key, "original-error").unwrap();
    delivery_receipt::confirm(db, &key, "900").unwrap();
    let pending = fixture
        .queue
        .stage_turn_completion(
            "thread-b",
            result.turn_id.as_deref().unwrap(),
            &"Saved answer ".repeat(220),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(worker(&fixture).deliver_one(&pending).await.is_err());
    let request = final_recovery::Request {
        delivery_id: pending.delivery_id.clone(),
        job_id: pending.job_id.clone(),
        thread_id: pending.target_thread_id.clone(),
        turn_id: pending.turn_id.clone(),
        channel_id: pending.channel_id,
        original_sha256: final_recovery::sha256(&pending.content),
        ingress_id: "message:801".into(),
        error_receipt_key: key,
        error_message_id: "900".into(),
        error_sha256: "original-error".into(),
    };
    final_recovery::authorize(db, &request, |s| {
        cdr_discord::text::split_delivery_chunks(s, true)
    })
    .unwrap();
    let frozen = delivery::list_pending(db).unwrap().remove(0);
    let expected = cdr_discord::text::split_delivery_chunks(&frozen.content, true);
    worker(&fixture).deliver_pending().await.unwrap();
    worker(&fixture).deliver_pending().await.unwrap();
    assert!(worker(&fixture).ensure_first_reply("saved").is_err());
    assert!(cdr_store::queue::list(db).unwrap().is_empty());
    fixture.server.close().await.unwrap();
    http.stop.send(()).unwrap();
    let posts = http.task.await.unwrap();
    assert_eq!(
        posts
            .iter()
            .map(|(_, v)| v["content"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        app_fixture::rpc_log(&temp.path().join("rpc.jsonl"))
            .iter()
            .filter(|r| r["method"] == "turn/start")
            .count(),
        1
    );
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
