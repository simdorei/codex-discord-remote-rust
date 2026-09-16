use super::*;
use crate::test_support::{
    cleanup_refusal_transport::RefusingTransport, http_gate, message_fixture::MessageFixture,
};
use std::{sync::atomic::Ordering, time::Duration};
use twilight_model::id::Id;

#[tokio::test]
async fn mc_6_message_known_refusal_is_saved_before_one_notice_and_confirmed_after_delivery() {
    check(None).await;
}

#[tokio::test]
async fn mc_7_message_receipt_failure_never_sends_generic_error_or_saved_notice() {
    check(Some("receipt")).await;
}

#[tokio::test]
async fn mc_7_message_confirmation_failure_preserves_delivered_refusal_without_reposting() {
    check(Some("confirm")).await;
}

async fn check(fault: Option<&str>) {
    let temp = tempfile::tempdir().unwrap();
    let gate = http_gate::start().await;
    let http = Arc::new(
        Client::builder()
            .proxy(gate.address, true)
            .ratelimiter(None)
            .build(),
    );
    let fixture = MessageFixture::new(&temp, http).await;
    let remote = Arc::new(RefusingTransport::default());
    fixture
        .executor
        .set_mirror_transport(remote.clone(), Some(1))
        .unwrap();
    let db = fixture.executor.mirror_db().to_path_buf();
    let connection = rusqlite::Connection::open(&db).unwrap();
    match fault {
        Some("receipt") => connection.execute_batch("CREATE TRIGGER reject_receipt BEFORE UPDATE OF message_id ON codex_delivery_receipts BEGIN SELECT RAISE(ABORT, 'injected receipt failure'); END;").unwrap(),
        Some("confirm") => connection.execute_batch("CREATE TRIGGER reject_confirm BEFORE UPDATE OF confirmation_delivered ON discord_ingress_journal BEGIN SELECT RAISE(ABORT, 'injected ingress confirm failure'); END;").unwrap(),
        None => (), _ => unreachable!(),
    }
    let admitted = fixture.admit("!mirror sync");
    let root = temp.path().to_path_buf();
    let worker = tokio::spawn(async move {
        let result = process_admitted_gateway_message(admitted, &fixture.context(&root)).await;
        if let Err(error) = result {
            report_processing_error(
                fixture.executor.mirror_db(),
                &DiscordHttp::new(fixture.http.clone(), Id::new(1)),
                ErrorReportTarget {
                    channel_id: Id::new(42),
                    message_id: Id::new(801),
                },
                error,
            )
            .await;
        }
        fixture
    });
    let observed = tokio::time::timeout(Duration::from_secs(3), gate.entered).await;
    assert!(observed.is_ok(), "known refusal must reach Discord");
    let before = cdr_store::ingress::get(&db, "message:801")
        .unwrap()
        .unwrap();
    assert_eq!(before.outcome.as_ref().unwrap()["sync_completed"], false);
    assert_eq!(before.outcome.as_ref().unwrap()["blocked_room_id"], 31);
    assert!(!before.confirmation_delivered);
    gate.release.send(()).unwrap();
    let fixture = tokio::time::timeout(Duration::from_secs(3), worker)
        .await
        .unwrap()
        .unwrap();
    let after = cdr_store::ingress::get(&db, "message:801")
        .unwrap()
        .unwrap();
    assert_eq!(after.outcome, before.outcome);
    assert_eq!(after.confirmation_delivered, fault.is_none());
    if fault == Some("confirm") {
        assert!(
            after
                .hold_reason
                .contains("delivery confirmed; ingress confirmation write failed"),
            "{}",
            after.hold_reason
        );
    }
    if fault == Some("receipt") {
        assert!(after.hold_reason.contains("delivery unconfirmed"));
    }
    cdr_store::ingress::recover_prior_runtime(&db, "next-runtime", 1000.0).unwrap();
    assert!(cdr_store::delivery::list_pending(&db).unwrap().is_empty());
    assert!(
        admit_message_candidate_at(
            fixture.classify_id("!mirror sync", 801),
            std::time::SystemTime::now()
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(remote.calls.load(Ordering::SeqCst), 1);
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    assert_eq!(posts.len(), 1);
    assert!(
        posts[0]["content"]
            .as_str()
            .unwrap()
            .contains("Mirror sync stopped")
    );
    assert!(
        !posts[0]["content"]
            .as_str()
            .unwrap()
            .contains("Mirror sync complete")
    );
    fixture.server.close().await.unwrap();
}
