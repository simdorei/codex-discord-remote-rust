use super::*;
use cdr_discord::components::{ApprovalAnswer, ComponentId};
use twilight_model::id::Id;

#[tokio::test]
async fn repeated_component_error_uses_confirmed_discord_receipt() {
    check_receipt(false).await;
}

#[tokio::test]
async fn component_error_unknown_receipt_is_preserved_without_reposting() {
    check_receipt(true).await;
}

async fn check_receipt(fail_confirmation: bool) {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    if fail_confirmation {
        cdr_store::schema::open_initialized(&db).unwrap().execute_batch(
            "CREATE TRIGGER reject_receipt BEFORE UPDATE OF message_id ON codex_delivery_receipts BEGIN SELECT RAISE(ABORT, 'injected receipt failure'); END;"
        ).unwrap();
    }
    let gate = crate::test_support::http_gate::start().await;
    let http = Arc::new(
        Client::builder()
            .proxy(gate.address, true)
            .ratelimiter(None)
            .build(),
    );
    let work = InboundInteractionWork {
        application_id: Id::new(2),
        interaction_id: Id::new(101),
        channel_id: Id::new(42),
        user_id: Id::new(3),
        source_message_id: Some(Id::new(301)),
        interaction_token: "fixture".into(),
        work: RoutedWork::Component(ComponentId::BoundApproval {
            thread_fingerprint: "0".repeat(16),
            request_fingerprint: "1".repeat(32),
            answer: ApprovalAnswer::Approve,
        }),
        processing_mode: crate::discord_dispatch::InteractionProcessingMode::Execute,
        custody_database: db.clone(),
        custody_ingress_id: "interaction:101".into(),
        authorized_busy_choice: None,
        admission_permit: None,
    };
    gate.release.send(()).unwrap();
    for _ in 0..2 {
        report_interaction_error(
            &work,
            InteractionWorkerError::Unsupported("fixture error"),
            http.clone(),
        )
        .await;
    }
    gate.entered.await.unwrap();
    gate.stop.send(()).unwrap();
    assert_eq!(
        gate.task.await.unwrap().len(),
        1,
        "same confirmed component error must not POST again"
    );
    assert_eq!(
        cdr_store::delivery_receipt::unknown_count(&db).unwrap(),
        i64::from(fail_confirmation)
    );
}
