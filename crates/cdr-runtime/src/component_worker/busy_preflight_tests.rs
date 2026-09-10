use super::busy::handle_busy_component;
use crate::{
    discord_dispatch::{InboundInteractionWork, InteractionProcessingMode},
    test_support::message_fixture::MessageFixture,
};
use cdr_app_server::requests::AppRequest;
use cdr_discord::{
    components::{BusyAction, ComponentId},
    interaction::RoutedWork,
};
use cdr_store::claims::{NewBusyChoice, create_busy_choice, get_busy_choice};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use twilight_model::id::Id;

fn choice(fixture: &MessageFixture) -> (ComponentId, InboundInteractionWork) {
    let db = fixture.executor.mirror_db();
    let generation = i64::try_from(fixture.server.generation()).unwrap();
    cdr_store::queue::enqueue(
        db,
        cdr_store::queue::NewQueueJob {
            job_id: "preceding",
            target_thread_id: "thread-b",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: None,
            app_server_generation: generation,
            prompt: "original",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    cdr_store::queue::begin_attempt(db, "preceding", &[], generation).unwrap();
    let now = super::now().unwrap();
    let choice_id = create_busy_choice(
        db,
        NewBusyChoice {
            owner_user_id: 3,
            channel_id: 42,
            target_thread_id: Some("thread-b"),
            prompt: "extra direction",
            allow_steer: false,
            now,
            time_to_live: 1800.0,
        },
    )
    .unwrap();
    cdr_store::control_binding::bind(db, &choice_id, "thread-b", None, Some("preceding")).unwrap();
    let stored = get_busy_choice(db, &choice_id, now).unwrap().unwrap();
    let component = ComponentId::Busy {
        choice_id,
        action: BusyAction::Steer,
    };
    let work = InboundInteractionWork {
        application_id: Id::new(1),
        interaction_id: Id::new(501),
        channel_id: Id::new(42),
        user_id: Id::new(3),
        source_message_id: Some(Id::new(502)),
        interaction_token: "fixture".into(),
        work: RoutedWork::Component(component.clone()),
        processing_mode: InteractionProcessingMode::Execute,
        custody_database: db.to_owned(),
        custody_ingress_id: "fixture-click".into(),
        authorized_busy_choice: Some(stored),
        admission_permit: None,
    };
    (component, work)
}

#[tokio::test]
async fn preparing_click_can_retry_original_turn_after_a_proven_pre_dispatch_rejection() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = MessageFixture::new(
        &temp,
        Arc::new(twilight_http::Client::new("fixture".into())),
    )
    .await;
    let (component, work) = choice(&fixture);
    let first = handle_busy_component(&work, &component, &fixture.executor, &fixture.server).await;
    assert!(first.is_err());
    let choice_id = &work.authorized_busy_choice.as_ref().unwrap().choice_id;
    let state = super::read_busy_choice_state(
        fixture.executor.mirror_db(),
        choice_id,
        super::now().unwrap(),
    )
    .unwrap()
    .unwrap();
    assert!(
        !state.claimed,
        "no control RPC was sent; preparing rejection must not permanently consume the button"
    );
    let generation = i64::try_from(fixture.server.generation()).unwrap();
    cdr_store::queue::mark_running(
        fixture.executor.mirror_db(),
        "preceding",
        "original-turn",
        generation,
    )
    .unwrap();
    fixture
        .server
        .execute(
            AppRequest {
                method: "test/active-turn",
                params: json!({"threadId":"thread-b","turnId":"original-turn"}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
    handle_busy_component(&work, &component, &fixture.executor, &fixture.server)
        .await
        .unwrap();
    // A repeated click reuses the durable confirmation, not another steer.
    handle_busy_component(&work, &component, &fixture.executor, &fixture.server)
        .await
        .unwrap();
    fixture.server.close().await.unwrap();
    let calls = crate::test_support::app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
    let steers: Vec<_> = calls
        .iter()
        .filter(|v| v["method"] == "turn/steer")
        .collect();
    assert_eq!(steers.len(), 1);
    assert_eq!(steers[0]["params"]["expectedTurnId"], "original-turn");
    assert!(!calls.iter().any(|v| matches!(
        v["method"].as_str(),
        Some("thread/fork" | "thread/resume" | "turn/start")
    )));
}
