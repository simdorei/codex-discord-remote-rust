use super::*;
use crate::{
    message_worker, new_reply_worker,
    test_support::{app_fixture, new_reply_fixture as support},
};
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

async fn delayed_first_reply(delay: Duration, ack_first: bool) {
    let temp = tempfile::tempdir().unwrap();
    let (fixture, remote, mut gate) = support::setup(&temp).await;
    let context = fixture.context(temp.path());
    let mut request = Box::pin(message_worker::process_admitted_gateway_message(
        fixture.admit("!new 첫 요청"),
        &context,
    ));
    tokio::time::timeout(Duration::from_secs(2),async {
        tokio::select! { result=&mut request=>panic!("first reply failed before HTTP: {result:?}"),entered=&mut gate.entered=>entered.unwrap() }
    }).await.expect("new acknowledgement waited for persistence");
    let db = fixture.executor.mirror_db();
    let job = cdr_store::queue::list(db)
        .unwrap()
        .into_iter()
        .find(|j| j.target_thread_id == "new-thread")
        .unwrap();
    let completion = worker(&fixture);
    let pending = completion
        .queue
        .stage_turn_completion("new-thread", "first-turn", "Final\n첫 답변")
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        completion.deliver_one(&pending).await,
        Err(CompletionWorkerError::Held(_))
    ));
    assert_eq!(delivery::list_pending(db).unwrap()[0].attempt_count, 0);
    let mut gate_release = Some(gate.release);
    let mut request = Some(request);
    if ack_first {
        gate_release.take().unwrap().send(()).unwrap();
        request.take().unwrap().await.unwrap();
    }
    tokio::time::sleep(delay).await;
    support::persist(&temp, &job.prompt);
    let records = new_reply_worker::reconcile(db).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].state, "verified");
    if !ack_first {
        gate_release.take().unwrap().send(()).unwrap();
        request.take().unwrap().await.unwrap();
    }
    let restarted = worker(&fixture);
    restarted.deliver_pending().await.unwrap();
    restarted.deliver_pending().await.unwrap();
    assert!(delivery::list_pending(db).unwrap().is_empty());
    assert!(
        new_reply::get(db, &job.job_id)
            .unwrap()
            .unwrap()
            .confirmation_delivered
    );
    assert_eq!(remote.creates.load(std::sync::atomic::Ordering::SeqCst), 1);
    fixture.server.close().await.unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    assert_eq!(posts.len(), 2);
    assert_eq!(
        posts[0]["content"],
        "In progress\nmessage: 첫 요청\n새 대화: <#43>"
    );
    assert_eq!(posts[0]["test_channel"], 42);
    assert_eq!(posts[1]["test_channel"], 43);
    assert_eq!(posts[1]["content"], "Final\n첫 답변");
    let rpc = app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
    assert_eq!(
        rpc.iter().filter(|r| r["method"] == "thread/start").count(),
        1
    );
    assert_eq!(
        rpc.iter().filter(|r| r["method"] == "turn/start").count(),
        1
    );
    assert_eq!(
        rpc.iter().filter(|r| r["method"] == "thread/fork").count(),
        0
    );
}

#[tokio::test]
async fn eight_second_persistence_delay_does_not_lose_first_final() {
    delayed_first_reply(Duration::from_secs(8), true).await;
}
#[tokio::test]
async fn thirty_second_persistence_delay_does_not_lose_first_final() {
    delayed_first_reply(Duration::from_secs(30), true).await;
}
#[tokio::test]
async fn early_completion_and_verification_wait_for_actual_ack_receipt() {
    delayed_first_reply(Duration::ZERO, false).await;
}

#[tokio::test]
async fn bare_new_then_plain_message_runs_one_first_turn_and_delivers_final() {
    let temp = tempfile::tempdir().unwrap();
    let (fixture, remote, mut gate) = support::setup(&temp).await;
    let context = fixture.context(temp.path());
    let mut arm = Box::pin(message_worker::process_admitted_gateway_message(
        fixture.admit_id("!new", 800),
        &context,
    ));
    tokio::time::timeout(Duration::from_secs(2), async {
        tokio::select! {result=&mut arm=>panic!("arm failed before reply: {result:?}"),entered=&mut gate.entered=>entered.unwrap()}
    }).await.unwrap();
    assert_eq!(remote.creates.load(std::sync::atomic::Ordering::SeqCst), 0);
    gate.release.send(()).unwrap();
    arm.await.unwrap();
    tokio::time::timeout(
        Duration::from_secs(2),
        message_worker::process_admitted_gateway_message(fixture.admit("첫 요청"), &context),
    )
    .await
    .unwrap()
    .unwrap();
    let db = fixture.executor.mirror_db();
    let job = cdr_store::queue::list(db)
        .unwrap()
        .into_iter()
        .find(|j| j.target_thread_id == "new-thread")
        .unwrap();
    support::persist(&temp, &job.prompt);
    assert_eq!(
        new_reply_worker::reconcile(db).unwrap()[0].state,
        "verified"
    );
    let completion = worker(&fixture);
    completion
        .queue
        .stage_turn_completion("new-thread", "first-turn", "Final\n첫 답변")
        .await
        .unwrap();
    completion.deliver_pending().await.unwrap();
    completion.deliver_pending().await.unwrap();
    assert_eq!(remote.creates.load(std::sync::atomic::Ordering::SeqCst), 1);
    fixture.server.close().await.unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    assert_eq!(posts.len(), 3);
    assert_eq!(
        posts[0]["content"],
        "새 대화를 준비했습니다. 같은 방에 첫 요청을 보내주세요."
    );
    assert_eq!(
        posts[1]["content"],
        "In progress\nmessage: 첫 요청\n새 대화: <#43>"
    );
    assert_eq!(posts[1]["test_channel"], 42);
    assert_eq!(posts[2]["content"], "Final\n첫 답변");
    assert_eq!(posts[2]["test_channel"], 43);
    let rpc = app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
    for method in ["thread/start", "turn/start"] {
        assert_eq!(rpc.iter().filter(|r| r["method"] == method).count(), 1);
    }
    assert_eq!(
        rpc.iter().filter(|r| r["method"] == "thread/fork").count(),
        0
    );
}
