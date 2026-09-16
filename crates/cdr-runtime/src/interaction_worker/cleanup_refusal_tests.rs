use super::*;
use crate::test_support::{
    cleanup_refusal_transport::RefusingTransport, http_gate, mapped_slash,
    message_fixture::MessageFixture,
};
use std::{sync::atomic::Ordering, time::Duration};

#[tokio::test]
async fn mc_6_slash_refusal_stays_false_through_normal_finish_with_only_one_patch() {
    check(false, false).await;
}

#[tokio::test]
async fn mc_7_slash_confirm_failure_does_not_overwrite_refusal_with_generic_error() {
    check(true, false).await;
}

#[tokio::test]
async fn mc_8_slash_interrupted_notification_does_not_replay_or_stage_saved_notice() {
    check(false, true).await;
}

async fn check(confirm_failure: bool, interrupt: bool) {
    let temp = tempfile::tempdir().unwrap();
    let gate = http_gate::start_for_slash_refusal().await;
    let http = Arc::new(
        Client::builder()
            .proxy(gate.address, true)
            .ratelimiter(None)
            .build(),
    );
    let fixture = MessageFixture::new(&temp, http.clone()).await;
    let remote = Arc::new(RefusingTransport::default());
    fixture
        .executor
        .set_mirror_transport(remote.clone(), Some(1))
        .unwrap();
    let db = fixture.executor.mirror_db().to_path_buf();
    let work = mapped_slash::stage(&db, "bridge_sync").await;
    if confirm_failure {
        rusqlite::Connection::open(&db).unwrap().execute_batch("CREATE TRIGGER reject_confirm BEFORE UPDATE OF confirmation_delivered ON discord_ingress_journal BEGIN SELECT RAISE(ABORT, 'injected ingress confirm failure'); END;").unwrap();
    }
    let server = fixture.server.clone();
    let executor = Arc::new(fixture.executor);
    let (send, receiver) = mpsc::channel(1);
    send.send(work).await.unwrap();
    drop(send);
    let worker = tokio::spawn(run_interaction_worker(
        receiver,
        executor,
        server.clone(),
        http,
    ));
    tokio::time::timeout(Duration::from_secs(3), gate.entered)
        .await
        .unwrap()
        .unwrap();
    let before = cdr_store::ingress::get(&db, "interaction:101")
        .unwrap()
        .unwrap();
    assert_eq!(
        before
            .outcome
            .as_ref()
            .expect("refusal is durable before PATCH")["sync_completed"],
        false
    );
    assert_eq!(before.outcome.as_ref().unwrap()["blocked_room_id"], 31);
    assert!(!before.confirmation_delivered);
    if interrupt {
        worker.abort();
    }
    gate.release.send(()).unwrap();
    let finished = tokio::time::timeout(Duration::from_secs(3), worker)
        .await
        .unwrap();
    assert_eq!(finished.is_err(), interrupt);
    let after = cdr_store::ingress::get(&db, "interaction:101")
        .unwrap()
        .unwrap();
    assert_eq!(after.outcome, before.outcome);
    assert_eq!(after.confirmation_delivered, !confirm_failure && !interrupt);
    if confirm_failure {
        assert!(
            after
                .hold_reason
                .contains("delivery confirmed; ingress confirmation write failed"),
            "{}",
            after.hold_reason
        );
    }
    cdr_store::ingress::recover_prior_runtime(&db, "new-runtime", 1000.0).unwrap();
    assert!(cdr_store::delivery::list_pending(&db).unwrap().is_empty());
    assert!(
        !cdr_store::ingress::begin_execution(&db, "interaction:101", "processing", None, 1001.0)
            .unwrap()
    );
    gate.stop.send(()).unwrap();
    let requests = gate.task.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["test_method"], "PATCH");
    assert!(
        requests[0]["content"]
            .as_str()
            .unwrap()
            .contains("Mirror sync stopped")
    );
    assert_eq!(remote.calls.load(Ordering::SeqCst), 1);
    server.close().await.unwrap();
}
