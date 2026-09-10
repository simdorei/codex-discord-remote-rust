use super::*;
use crate::test_support::message_fixture::MessageFixture;
use crate::test_support::pro_fixture;
use crate::{
    component_worker::{BusyComponentError, handle_component_work, read_busy_choice_state},
    discord_dispatch::{InboundInteractionWork, InteractionProcessingMode},
};
use cdr_discord::{
    components::{BusyAction, ComponentId},
    interaction::RoutedWork,
};
use serde_json::json;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

async fn assert_steer_rejected(
    fixture: &MessageFixture,
    components: &[serde_json::Value],
) -> (InboundInteractionWork, String) {
    let queue = components
        .iter()
        .find_map(|b| b["custom_id"].as_str().filter(|id| id.ends_with(":queue")))
        .unwrap();
    let Some(ComponentId::Busy { choice_id, .. }) =
        cdr_discord::components::parse_component_id(queue)
    else {
        panic!("queue button");
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let state = read_busy_choice_state(fixture.executor.mirror_db(), &choice_id, now)
        .unwrap()
        .unwrap();
    let component = ComponentId::Busy {
        choice_id: choice_id.clone(),
        action: BusyAction::Steer,
    };
    let work = InboundInteractionWork {
        application_id: Id::new(1),
        interaction_id: Id::new(901),
        channel_id: Id::new(42),
        user_id: Id::new(3),
        source_message_id: Some(Id::new(1)),
        interaction_token: "fixture-unused".into(),
        work: RoutedWork::Component(component.clone()),
        processing_mode: InteractionProcessingMode::Execute,
        custody_database: fixture.executor.mirror_db().into(),
        custody_ingress_id: "interaction:901".into(),
        authorized_busy_choice: Some(state.choice),
        admission_permit: None,
    };
    let result = handle_component_work(&work, &component, &fixture.executor, &fixture.server).await;
    assert!(
        matches!(
            result,
            Err(ComponentWorkerError::Busy(
                BusyComponentError::ControlNotDispatched(_)
            ))
        ),
        "Pro steer not explicitly rejected"
    );
    assert!(
        !read_busy_choice_state(fixture.executor.mirror_db(), &choice_id, now)
            .unwrap()
            .unwrap()
            .claimed
    );
    assert!(
        !components.iter().any(|b| b["custom_id"]
            .as_str()
            .is_some_and(|id| id.ends_with(":steer"))),
        "unsupported Pro steer is still shown"
    );
    (work, choice_id)
}

#[tokio::test]
async fn actual_pro_busy_click_is_rejected_without_dispatch_or_claim() {
    for (raw, case) in [
        ("!pro 확인", "healthy"),
        ("!pro review 검수", "healthy"),
        ("!pro review 검수", "offline"),
        ("!pro review 검수", "invalid-plugin"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let gate = crate::test_support::http_gate::start().await;
        let mut fixture = MessageFixture::new(
            &temp,
            Arc::new(
                Client::builder()
                    .proxy(gate.address, true)
                    .ratelimiter(None)
                    .build(),
            ),
        )
        .await;
        let connected = pro_fixture::configure(&temp, &mut fixture, case).await;
        fixture
            .server
            .request(
                "test/active-turn",
                json!({"threadId":"thread-b","turnId":"turn-b"}),
                Duration::from_secs(2),
                None,
            )
            .await
            .unwrap();
        gate.release.send(()).unwrap();
        process_admitted_gateway_message(fixture.admit(raw), &fixture.context(temp.path()))
            .await
            .unwrap();
        gate.stop.send(()).unwrap();
        let posts = gate.task.await.unwrap();
        let components = posts[0]["components"][0]["components"].as_array().unwrap();
        let (mut work, choice_id) = assert_steer_rejected(&fixture, components).await;
        let before = crate::test_support::app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
        assert!(!before.iter().any(|v| matches!(
            v["method"].as_str(),
            Some("turn/steer" | "turn/start" | "thread/fork" | "thread/start")
        )));
        fixture
            .server
            .request(
                "test/finish-turn",
                json!({"threadId":"thread-b","turnId":"turn-b"}),
                Duration::from_secs(2),
                None,
            )
            .await
            .unwrap();
        let component = ComponentId::Busy {
            choice_id,
            action: BusyAction::Queue,
        };
        work.work = RoutedWork::Component(component.clone());
        let queued =
            handle_component_work(&work, &component, &fixture.executor, &fixture.server).await;
        assert_eq!(
            queued.is_ok(),
            case == "healthy",
            "queue result disagrees with actual Pro availability: {case}"
        );
        if let Some(connected) = connected {
            connected.close().await;
        }
        fixture.server.close().await.unwrap();
        let calls = crate::test_support::app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
        assert!(!calls.iter().any(|v| matches!(
            v["method"].as_str(),
            Some("turn/steer" | "thread/fork" | "thread/start")
        )));
        let starts: Vec<_> = calls
            .iter()
            .filter(|v| v["method"] == "turn/start")
            .collect();
        assert_eq!(starts.len(), usize::from(case == "healthy"));
        if let Some(start) = starts.first() {
            assert_eq!(start["params"]["threadId"], "thread-b");
            let input = start["params"]["input"].as_array().unwrap();
            assert_eq!(input.len(), 3);
            let text = input[0]["text"].as_str().unwrap();
            assert!(text.contains("<local-device-mcp"));
            assert_eq!(text.contains("<pro-review>"), raw.contains("review"));
            assert!(text.contains(temp.path().join("original-project").to_str().unwrap()));
        }
    }
}
