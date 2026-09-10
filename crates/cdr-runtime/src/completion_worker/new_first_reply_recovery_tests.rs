use super::*;
use crate::{message_worker, new_reply_worker, test_support::new_reply_fixture as support};
use cdr_store::{delivery, new_reply};

fn worker(fixture: &crate::test_support::message_fixture::MessageFixture) -> CompletionWorker {
    CompletionWorker {
        server: fixture.server.clone(),
        queue: fixture.queue.clone(),
        http: fixture.http.clone(),
        commentary_enabled: true,
        history_read_timeout: Duration::from_secs(1),
        commentary: Mutex::new(CommentaryBuffer::default()),
        terminal_fence: terminal_fence::TerminalFence::default(),
    }
}

#[tokio::test]
async fn accepted_then_result_write_failure_recovers_normal_ack_and_existing_final_without_rpc_replay()
 {
    let temp = tempfile::tempdir().unwrap();
    let (fixture, remote, mut gate) = support::setup(&temp).await;
    let db = fixture.executor.mirror_db();
    let sql = rusqlite::Connection::open(db).unwrap();
    sql.execute_batch("CREATE TRIGGER reject_result BEFORE UPDATE OF phase ON discord_ingress_journal
        WHEN NEW.phase='result_recorded' BEGIN SELECT RAISE(ABORT,'injected result recording failure'); END;").unwrap();
    let context = fixture.context(temp.path());
    let failed =
        message_worker::process_admitted_gateway_message(fixture.admit("!new 복구 요청"), &context)
            .await
            .unwrap_err();
    assert!(
        failed
            .to_string()
            .contains("injected result recording failure")
    );
    let job = cdr_store::queue::list(db).unwrap().remove(0);
    let record = new_reply::get(db, &job.job_id).unwrap().unwrap();
    assert!(record.acknowledgement_recovery_allowed);
    assert!(!record.confirmation_delivered);
    assert_eq!(
        sql.query_row("SELECT COUNT(*) FROM codex_delivery_receipts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let completion = Arc::new(worker(&fixture));
    completion
        .queue
        .stage_turn_completion("new-thread", "first-turn", "Final\n복구 답변")
        .await
        .unwrap();
    assert!(cdr_store::queue::list(db).unwrap().is_empty());
    support::persist(&temp, &job.prompt);
    sql.execute_batch("DROP TRIGGER reject_result;").unwrap();
    let (shutdown, receiver) = watch::channel(false);
    let recovery = tokio::spawn(new_reply_worker::run(
        db.into(),
        fixture.queue.clone(),
        fixture.http.clone(),
        receiver,
    ));
    let (events, pending) = tokio::sync::mpsc::channel(4);
    let processing = Arc::clone(&completion);
    let output = tokio::spawn(async move { super::driver::process(&processing, pending).await });
    tokio::time::timeout(Duration::from_secs(3), &mut gate.entered)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(delivery::list_pending(db).unwrap().len(), 1);
    gate.release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !delivery::list_pending(db).unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("durable acknowledgement/verification did not wake the completion worker");
    shutdown.send(true).unwrap();
    recovery.await.unwrap();
    drop(events);
    output.await.unwrap();
    fixture.server.close().await.unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    assert_eq!(posts.len(), 2);
    assert_eq!(
        posts[0]["content"],
        "In progress\nmessage: 복구 요청\n새 대화: <#43>"
    );
    assert_eq!(posts[1]["content"], "Final\n복구 답변");
    assert_eq!(remote.creates.load(std::sync::atomic::Ordering::SeqCst), 1);
    let rpc = crate::test_support::app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
    assert_eq!(
        rpc.iter().filter(|r| r["method"] == "turn/start").count(),
        1
    );
    assert_eq!(
        rpc.iter().filter(|r| r["method"] == "thread/start").count(),
        1
    );
}

#[tokio::test]
async fn unknown_ack_after_worker_death_does_not_resend_or_release_final() {
    let temp = tempfile::tempdir().unwrap();
    let (fixture, _, mut gate) = support::setup(&temp).await;
    let context = fixture.context(temp.path());
    let db = fixture.executor.mirror_db();
    let mut first = Box::pin(message_worker::process_admitted_gateway_message(
        fixture.admit("!new 중복 금지"),
        &context,
    ));
    tokio::select! { result=&mut first=>panic!("first acknowledgement failed: {result:?}"),entered=&mut gate.entered=>entered.unwrap() }
    let job = cdr_store::queue::list(db).unwrap().remove(0);
    let completion = worker(&fixture);
    completion
        .queue
        .stage_turn_completion("new-thread", "first-turn", "Final\nanswer")
        .await
        .unwrap();
    drop(first);
    gate.release.send(()).unwrap();
    support::persist(&temp, &job.prompt);
    let (shutdown, receiver) = watch::channel(false);
    let recovery = tokio::spawn(new_reply_worker::run(
        db.into(),
        fixture.queue.clone(),
        fixture.http.clone(),
        receiver,
    ));
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(matches!(
        completion.deliver_pending().await,
        Err(CompletionWorkerError::Held(_))
    ));
    assert_eq!(delivery::list_pending(db).unwrap()[0].attempt_count, 0);
    assert!(
        !new_reply::get(db, &job.job_id)
            .unwrap()
            .unwrap()
            .confirmation_delivered
    );
    shutdown.send(true).unwrap();
    recovery.await.unwrap();
    fixture.server.close().await.unwrap();
    gate.stop.send(()).unwrap();
    assert_eq!(gate.task.await.unwrap().len(), 1);
}
