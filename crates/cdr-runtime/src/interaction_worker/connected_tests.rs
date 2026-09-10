use std::sync::{Arc, atomic::Ordering};

use cdr_discord::components::{BusyAction, ComponentId};
use cdr_store::claims::{NewBusyChoice, create_busy_choice};
use cdr_store::prompt_intake::{admit_busy_queue, list_prompt_intakes};
use cdr_store::queue::list;
use rusqlite::{Connection, params};
use twilight_http::Client;

use super::*;
use crate::component_worker::{BusyComponentError, ComponentWorkerError, ConfirmationError};
use crate::discord_dispatch::InteractionProcessingMode;

#[path = "connected_tests/support.rs"]
mod support;
use support::*;

#[test]
fn indeterminate_actions_are_never_promoted_to_known_success() {
    let standard = ComponentWorkerError::ActionOutcomeIndeterminate("unknown".into());
    let busy = ComponentWorkerError::Busy(BusyComponentError::ActionOutcomeIndeterminate(
        "unknown".into(),
    ));
    let post_success =
        ComponentWorkerError::Confirmation(ConfirmationError::Recovery("marker failed".into()));

    assert!(!standard.action_completed_before_failure());
    assert!(!busy.action_completed_before_failure());
    assert!(post_success.action_completed_before_failure());
}

#[tokio::test]
async fn marker_write_failure_after_external_success_stays_known_and_unconfirmed() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("mirror.sqlite");
    stage(&database, "interaction:41", 41);
    reject_confirmation_ready_writes(&database);
    let (server, log) = start_fake_server(&temp).await;
    let component = owned_approval(&database, &server).await;
    assert_eq!(server.pending_server_requests(None).await.unwrap().len(), 1);
    let backend = Arc::new(CountingBackend::default());
    let executor = Arc::new(executor(&temp, database.clone(), Arc::clone(&backend)));
    let (sender, receiver) = mpsc::channel(1);
    sender
        .send(work(
            &database,
            "interaction:41",
            41,
            component,
            InteractionProcessingMode::Execute,
            None,
        ))
        .await
        .unwrap();
    drop(sender);

    run_interaction_worker(
        receiver,
        executor,
        Arc::clone(&server),
        Arc::new(Client::new("not-used".into())),
    )
    .await;

    let row = cdr_store::ingress::get(&database, "interaction:41")
        .unwrap()
        .unwrap();
    assert_eq!(approval_response_count(&log), 1);
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    assert_eq!(row.state, "completed");
    assert_eq!(row.phase, "result_recorded");
    assert!(!row.confirmation_delivered);
    assert_eq!(row.hold_reason, "");
    assert_eq!(row.outcome.as_ref().unwrap()["action_completed"], true);
    assert!(
        row.outcome.as_ref().unwrap()["confirmation_error"]
            .as_str()
            .unwrap()
            .contains("fixture rejected confirmation-ready marker")
    );
    server.close().await.unwrap();
}

#[tokio::test]
async fn confirmation_only_worker_fails_closed_without_another_queue_or_backend_call() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("mirror.sqlite");
    let choice_id = create_busy_choice(
        &database,
        NewBusyChoice {
            owner_user_id: 20,
            channel_id: 10,
            target_thread_id: Some("thread-a"),
            prompt: "queue exactly once",
            allow_steer: true,
            now: 1.0,
            time_to_live: 1_800.0,
        },
    )
    .unwrap();
    let first = admit_busy(&database, "interaction:51", 51, &choice_id);
    let choice = first.busy_choice.unwrap();
    assert!(cdr_store::ingress::acknowledge(&database, "interaction:51", 2.0).unwrap());
    let ready = crate::component_worker::busy_ready_marker(&choice_id, 20, 10);
    assert!(
        admit_busy_queue(&database, &choice, "thread-a", false, &ready, 3.0)
            .unwrap()
            .intake
            .is_some()
    );
    let repeat = admit_busy(&database, "interaction:52", 52, &choice_id);
    assert!(repeat.canonical_repeat_created);
    Connection::open(&database)
        .unwrap()
        .execute(
            "DELETE FROM persistent_component_claims WHERE claim_key=?",
            params![ready],
        )
        .unwrap();
    let backend = Arc::new(CountingBackend::default());
    let executor = executor(&temp, database.clone(), Arc::clone(&backend));
    let (server, _) = start_fake_server(&temp).await;
    let work = work(
        &database,
        "interaction:52",
        52,
        ComponentId::Busy {
            choice_id,
            action: BusyAction::Queue,
        },
        InteractionProcessingMode::ConfirmationOnly,
        repeat.busy_choice,
    );
    let mut custody = ExecutionCustody::begin(
        &database,
        &database,
        "interaction:52",
        InteractionProcessingMode::ConfirmationOnly,
    )
    .unwrap();

    let error = process_interaction_work(
        &work,
        &executor,
        &server,
        Arc::new(Client::new("not-used".into())),
        &mut custody,
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        InteractionWorkerError::Component(ComponentWorkerError::Busy(
            BusyComponentError::ActionUnconfirmed
        ))
    ));
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    assert!(list(&database).unwrap().is_empty());
    assert_eq!(list_prompt_intakes(&database).unwrap().len(), 1);
    server.close().await.unwrap();
}
