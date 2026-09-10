use super::*;
use crate::discord_dispatch::InteractionProcessingMode;

async fn observe_turn(fixture: &MessageFixture, method: &str) {
    fixture
        .server
        .request(
            method,
            json!({"threadId":"thread-b","turnId":"original-turn"}),
            Duration::from_secs(2),
            None,
        )
        .await
        .unwrap();
}

async fn reject_and_record(
    fixture: &MessageFixture,
    work: &InboundInteractionWork,
    http: Arc<Client>,
) {
    let db = fixture.executor.mirror_db();
    let mut custody =
        ExecutionCustody::begin(db, db, &work.custody_ingress_id, work.processing_mode).unwrap();
    let error = process_interaction_work(
        work,
        &fixture.executor,
        &fixture.server,
        http.clone(),
        &mut custody,
    )
    .await
    .unwrap_err();
    custody.hold_failed().unwrap();
    let row = cdr_store::ingress::get(db, "interaction:701")
        .unwrap()
        .unwrap();
    assert_eq!(
        row.outcome.as_ref().map(|v| &v["control_dispatched"]),
        Some(&json!(false)),
        "real worker must durably distinguish an unsent Pro control: {error}"
    );
    assert_eq!(row.state, "completed");
}

#[tokio::test]
async fn old_pro_steer_rejection_allows_a_new_explicit_queue_click_without_replay() {
    for case in ["healthy", "offline", "invalid-plugin"] {
        let temp = tempfile::tempdir().unwrap();
        let gate = http_gate::start().await;
        let http = Arc::new(
            Client::builder()
                .proxy(gate.address, true)
                .ratelimiter(None)
                .build(),
        );
        gate.release.send(()).unwrap();
        let mut fixture = MessageFixture::new(&temp, http.clone()).await;
        let connected =
            crate::test_support::pro_fixture::configure(&temp, &mut fixture, case).await;
        observe_turn(&fixture, "test/active-turn").await;
        let db = fixture.executor.mirror_db();
        let choice = create_busy_choice(
            db,
            NewBusyChoice {
                owner_user_id: 3,
                channel_id: 42,
                target_thread_id: Some("thread-b"),
                prompt: "!pro review 검수",
                allow_steer: true,
                now: crate::component_worker::now().unwrap(),
                time_to_live: 1800.0,
            },
        )
        .unwrap();
        cdr_store::control_binding::bind(db, &choice, "thread-b", Some("original-turn"), None)
            .unwrap();
        let (sender, mut receiver) = mpsc::channel(4);
        let dispatcher = InteractionDispatcher::new(
            Arc::new(Ack),
            InteractionAccessPolicy {
                allow_all_channels: true,
                ..Default::default()
            },
            false,
            sender,
            db,
        );
        assert_eq!(
            click_action(&dispatcher, &choice, 701, "steer").await,
            DispatchOutcome::Queued
        );
        let work = receiver.recv().await.unwrap();
        reject_and_record(&fixture, &work, http.clone()).await;
        observe_turn(&fixture, "test/finish-turn").await;
        assert_eq!(
            click_action(&dispatcher, &choice, 702, "queue").await,
            DispatchOutcome::Queued
        );
        let retry = receiver.recv().await.unwrap();
        assert_eq!(retry.processing_mode, InteractionProcessingMode::Execute);
        let replay = click_action(&dispatcher, &choice, 701, "steer").await;
        let (first_extra, second_extra) = tokio::join!(
            click_action(&dispatcher, &choice, 703, "queue"),
            click_action(&dispatcher, &choice, 704, "queue")
        );
        for outcome in [replay, first_extra, second_extra] {
            if outcome == DispatchOutcome::Queued {
                assert_ne!(
                    receiver.recv().await.unwrap().processing_mode,
                    InteractionProcessingMode::Execute
                );
            }
        }
        let mut custody =
            ExecutionCustody::begin(db, db, &retry.custody_ingress_id, retry.processing_mode)
                .unwrap();
        let result = process_interaction_work(
            &retry,
            &fixture.executor,
            &fixture.server,
            http.clone(),
            &mut custody,
        )
        .await;
        assert_eq!(result.is_ok(), case == "healthy", "{case}");
        if result.is_ok() {
            custody
                .finish_success(&json!({"action_completed":true}))
                .unwrap();
        } else {
            custody.hold_failed().unwrap();
        }
        gate.stop.send(()).unwrap();
        let _ = gate.task.await.unwrap();
        if let Some(connected) = connected {
            connected.close().await;
        }
        fixture.server.close().await.unwrap();
        assert_calls(&temp, case);
    }
}

async fn click_action(
    dispatcher: &InteractionDispatcher<Ack>,
    choice: &str,
    id: u64,
    action: &str,
) -> DispatchOutcome {
    let event=serde_json::from_value(json!({
        "application_id":"2","authorizing_integration_owners":{},"channel_id":"42",
        "data":{"component_type":2,"custom_id":format!("codex_busy:{choice}:{action}")},
        "entitlements":[],"id":id.to_string(),"locale":"ko","token":"fixture","type":3,
        "user":{"avatar":null,"bot":false,"discriminator":"0001","id":"3","username":"fixture"},"version":1
    })).unwrap();
    dispatcher
        .dispatch(
            &event,
            tokio::time::Instant::now(),
            InteractionIngressTag::Normal,
        )
        .await
        .unwrap()
}

fn assert_calls(temp: &tempfile::TempDir, case: &str) {
    let frames = app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
    assert!(!frames.iter().any(|v| matches!(
        v["method"].as_str(),
        Some("turn/steer" | "thread/fork" | "thread/start")
    )));
    let starts: Vec<_> = frames
        .iter()
        .filter(|v| v["method"] == "turn/start")
        .collect();
    assert_eq!(starts.len(), usize::from(case == "healthy"));
    if let Some(start) = starts.first() {
        assert_eq!(start["params"]["threadId"], "thread-b");
        let input = start["params"]["input"].as_array().unwrap();
        assert_eq!(input.len(), 3);
        let text = input[0]["text"].as_str().unwrap();
        assert!(text.contains("<pro-review>") && text.contains("<local-device-mcp"));
        assert!(text.contains(temp.path().join("original-project").to_str().unwrap()));
    }
}
