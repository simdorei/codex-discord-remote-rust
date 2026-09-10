use super::*;

#[tokio::test]
async fn wrong_project_acceptance_stays_unverified_on_replay_and_executor_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let state = temp.path().join("state.sqlite");
    let db = temp.path().join("mirror.sqlite");
    let cwd = temp.path().to_string_lossy().into_owned();
    let server = Arc::new(support::start_persisting_server(&temp, &log, &state).await);
    persistence::setup_project(&state, &db, &cwd, 99);
    server
        .execute(
            cdr_app_server::requests::AppRequest {
                method: "test/persist-new-in-other-project",
                params: serde_json::json!({"cwd":temp.path().join("wrong-project")}),
                timeout: std::time::Duration::from_secs(2),
            },
            Some(server.generation()),
        )
        .await
        .unwrap();
    let remote = Arc::new(Remote::default());
    let make_executor = || {
        let queue = Arc::new(QueueCoordinator::new(
            db.clone(),
            Arc::new(AppServerTurnBackend::new(server.clone())),
        ));
        let executor = ActionExecutor::new(
            state.clone(),
            db.clone(),
            Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
            queue,
        )
        .with_server(server.clone());
        executor
            .set_mirror_transport(remote.clone(), Some(1))
            .unwrap();
        executor
    };
    let executor = make_executor();
    let action = CommandAction::New {
        prompt: "새 작업".into(),
    };
    let first = executor.execute_with_context(action.clone(), CONTEXT).await;
    let accepted = first.unwrap();
    assert!(accepted.text.starts_with("In progress\nmessage:"));
    let original = cdr_store::ingress::by_origin(&db, 30).unwrap().unwrap();
    assert_eq!(original.target_thread_id.as_deref(), Some("new-thread"));
    assert!(original.owner_id.is_some());
    // A normal gateway result write must retain the immutable creation context.
    cdr_store::ingress::record_result(
        &db,
        &original.ingress_id,
        &serde_json::json!({"response":accepted.text}),
        60.0,
    )
    .unwrap();
    let replay = executor.execute_with_context(action.clone(), CONTEXT).await;
    assert_eq!(replay.unwrap().text, accepted.text);
    let job = original.owner_id.as_deref().unwrap();
    assert_eq!(
        cdr_store::new_reply::get(&db, job).unwrap().unwrap().state,
        "pending"
    );
    assert!(
        cdr_store::new_reply::output_hold(&db, job)
            .unwrap()
            .is_some()
    );
    drop(executor);
    let reopened = make_executor();
    let replay = reopened.execute_with_context(action.clone(), CONTEXT).await;
    assert_eq!(replay.unwrap().text, accepted.text);
    assert!(
        cdr_store::new_reply::output_hold(&db, job)
            .unwrap()
            .is_some()
    );
    // Legacy evidence is explicitly unverified; it never permits recreation.
    rusqlite::Connection::open(&db).unwrap().execute(
        "UPDATE discord_ingress_journal SET outcome_json=json_remove(outcome_json,'$.new_creation') WHERE ingress_id=?",
        [&original.ingress_id],
    ).unwrap();
    let legacy = reopened.execute_with_context(action, CONTEXT).await;
    assert!(
        legacy
            .unwrap_err()
            .to_string()
            .contains("evidence or original room mapping changed")
    );
    let retained = cdr_store::ingress::by_origin(&db, 30).unwrap().unwrap();
    assert_eq!(retained.owner_id, original.owner_id);
    assert_eq!(retained.target_thread_id, original.target_thread_id);
    assert_eq!(retained.payload, original.payload);
    assert_eq!(
        mirrored_thread_id(&db, Some(100)).unwrap().as_deref(),
        Some("new-thread")
    );
    assert_eq!(*remote.creates.lock().unwrap(), 1);
    persistence::verify_rpc_counts(&log, &cwd, false);
    server.close().await.unwrap();
}
