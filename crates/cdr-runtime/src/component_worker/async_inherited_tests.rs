//! R1 end-to-end: real resident notification -> HTTP controls -> exact steer.
use super::*;

async fn inherited_question(f: &MessageFixture, early: bool) -> (String, InboundInteractionWork) {
    let db = f.executor.mirror_db();
    queue::enqueue(
        db,
        queue::NewQueueJob {
            job_id: "origin",
            target_thread_id: "thread-b",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: Some(40),
            app_server_generation: 0,
            prompt: "original historical input",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    queue::begin_attempt(db, "origin", &[], 0).unwrap();
    queue::mark_running(db, "origin", "past-turn", 0).unwrap();
    queue::mark_goal_waiting(db, "origin", "past-turn", 0).unwrap();
    let before = queue::list(db).unwrap().remove(0);
    if !early {
        assert!(queue::attach_goal_turn_observed_if_owned(db, &before, "original", 1).unwrap());
    }
    let mut receiver = f.server.subscribe_notifications();
    control(f, "test/question").await;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(3), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        if let ResidentNotificationEvent::Notification {
            generation,
            notification,
        } = event
            && notification.method == "item/completed"
        {
            crate::async_question_ui::observe(
                db,
                f.server.instance_id(),
                generation,
                &notification.params,
            )
            .unwrap();
            break;
        }
    }
    let id = aq::occurrence_id("thread-b", "original", "question-call", 1).unwrap();
    if early {
        assert!(
            aq::get(db, &id).is_err(),
            "a candidate must not expose controls"
        );
        assert!(queue::attach_goal_turn_observed_if_owned(db, &before, "original", 1).unwrap());
    }
    crate::async_question_ui::deliver_pending(db, f.server.instance_id(), 1, &f.http)
        .await
        .unwrap();
    let q = aq::get(db, &id).unwrap();
    assert_eq!(q.state, "open");
    assert_eq!(q.generation, 1);
    let component = ComponentId::AsyncChoice {
        question_id: id.clone(),
        option: 1,
    };
    let work = InboundInteractionWork {
        application_id: Id::new(1),
        interaction_id: Id::new(601),
        channel_id: Id::new(42),
        user_id: Id::new(3),
        source_message_id: Some(Id::new(q.message_id.unwrap().parse().unwrap())),
        interaction_token: "fixture".into(),
        work: cdr_discord::interaction::RoutedWork::Component(component),
        processing_mode: InteractionProcessingMode::Execute,
        custody_database: db.to_owned(),
        custody_ingress_id: "inherited-fixture-click".into(),
        authorized_busy_choice: None,
        admission_permit: None,
    };
    (id, work)
}

#[tokio::test]
async fn inherited_question_early_and_attached_paths_deliver_and_steer_once_without_start_or_settings()
 {
    for early in [false, true] {
        let http = approval_http::start().await;
        let temp = tempfile::tempdir().unwrap();
        let f = fixture(&temp, http_client(&http.address)).await;
        control(&f, "test/active-turn").await;
        let (id, work) = inherited_question(&f, early).await;
        let before = queue::list(f.executor.mirror_db()).unwrap();
        assert_eq!(before[0].app_server_generation, 0);
        assert_eq!(before[0].execution_generation, Some(0));
        assert_eq!(before[0].turn_observation_generation, Some(1));
        let component = ComponentId::AsyncChoice {
            question_id: id.clone(),
            option: 1,
        };
        let first = handle_component_work(&work, &component, &f.executor, &f.server).await;
        assert!(first.is_ok(), "{:?}", first.err().map(|e| e.to_string()));
        assert!(
            handle_component_work(&work, &component, &f.executor, &f.server)
                .await
                .is_ok()
        );
        assert_eq!(
            aq::get(f.executor.mirror_db(), &id).unwrap().state,
            "submitted"
        );
        assert_eq!(queue::list(f.executor.mirror_db()).unwrap(), before);
        let other = aq::occurrence_id("thread-b", "original", "question-call", 0).unwrap();
        assert_eq!(
            aq::get(f.executor.mirror_db(), &other).unwrap().state,
            "open"
        );
        f.server.close().await.unwrap();
        http.stop.send(()).unwrap();
        let traffic = http.task.await.unwrap();
        assert_eq!(
            traffic
                .iter()
                .filter(|(_, body)| body["components"].as_array().is_some_and(|a| !a.is_empty()))
                .count(),
            2
        );
        let rpc = app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
        let steers: Vec<_> = rpc.iter().filter(|v| v["method"] == "turn/steer").collect();
        assert_eq!(steers.len(), 1);
        assert_eq!(steers[0]["params"]["expectedTurnId"], "original");
        assert!(!rpc.iter().any(|v| matches!(
            v["method"].as_str(),
            Some(
                "turn/start"
                    | "thread/resume"
                    | "thread/fork"
                    | "thread/settings/update"
                    | "account/rateLimits/read"
            )
        )));
    }
}

#[tokio::test]
async fn inherited_question_other_actor_and_same_numeric_generation_new_resident_cannot_answer() {
    let http = approval_http::start().await;
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp, http_client(&http.address)).await;
    control(&f, "test/active-turn").await;
    let (id, mut work) = inherited_question(&f, false).await;
    let component = ComponentId::AsyncChoice {
        question_id: id.clone(),
        option: 1,
    };
    work.user_id = Id::new(99);
    assert!(
        handle_component_work(&work, &component, &f.executor, &f.server)
            .await
            .is_err()
    );
    work.user_id = Id::new(3);
    work.channel_id = Id::new(99);
    assert!(
        handle_component_work(&work, &component, &f.executor, &f.server)
            .await
            .is_err()
    );
    work.channel_id = Id::new(42);
    f.server.close().await.unwrap();
    let replacement = start_server(&temp).await;
    assert_ne!(replacement.instance_id(), f.server.instance_id());
    assert!(
        handle_component_work(&work, &component, &f.executor, &replacement)
            .await
            .is_err()
    );
    assert_eq!(aq::get(f.executor.mirror_db(), &id).unwrap().state, "open");
    replacement.close().await.unwrap();
    http.stop.send(()).unwrap();
    http.task.await.unwrap();
    let rpc = app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
    assert!(!rpc.iter().any(|v| matches!(
        v["method"].as_str(),
        Some("turn/steer" | "turn/start" | "thread/settings/update")
    )));
}
