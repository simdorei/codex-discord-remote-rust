use super::*;

fn owning_queue(f: &HistoryFixture) -> QueueCoordinator<crate::app_backend::AppServerTurnBackend> {
    QueueCoordinator::new_with_admission_gate(
        f.db.clone(),
        f.backend.clone(),
        crate::restart_readiness::drain::AdmissionGate::new(),
    )
}

fn terminal_script(status: &str, input: Option<Value>, goal: Value) -> Value {
    let mut value = script(status, input);
    value["goal_result"] = goal;
    value
}

fn settlement_proof(f: &HistoryFixture) -> Option<Value> {
    cdr_store::schema::open_initialized(&f.db)
        .unwrap()
        .query_row(
            "SELECT proof_json FROM cdr_async_terminal_settlements WHERE question_id=?",
            [&f.id],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .unwrap()
        .map(|s| serde_json::from_str(&s).unwrap())
}

fn assert_read_only(f: &HistoryFixture, before: &[queue::StoredQueueJob]) {
    assert_eq!(queue::list(&f.db).unwrap(), before);
    assert!(
        f.calls().iter().all(|v| matches!(
            v["method"].as_str(),
            Some("initialize" | "thread/read" | "thread/turns/list" | "thread/goal/get")
        )),
        "historical reconciliation emitted a mutation"
    );
    assert_eq!(
        aq::get(&f.db, &f.id).unwrap().error,
        "original send timeout"
    );
}

#[tokio::test]
async fn orphan_terminal_without_goal_settles_ordinary_without_pending_or_replay() {
    let f = HistoryFixture::new(&terminal_script(
        "completed",
        Some(user_input(0)),
        json!({"goal":null}),
    ))
    .await;
    queue::complete(&f.db, "next").unwrap();
    let report = owning_queue(&f).recover_target("thread-b").await.unwrap();
    assert_eq!(report.started, 0);
    assert_eq!(
        f.obligation(),
        (
            "exact_history_confirmed".into(),
            "terminal".into(),
            "settled".into(),
            "original send timeout".into(),
            1
        )
    );
    assert!(!cdr_store::async_resolution::admission_held(&f.db, "thread-b").unwrap());
    assert_eq!(aq::get(&f.db, &f.id).unwrap().state, "closed_unknown");
    let proof = settlement_proof(&f).expect("historical terminal certificate was not saved");
    assert_eq!(proof["source"], "historical_read_terminal_v1");
    assert_ne!(proof["source"], "resident_notification_v1");
    for _ in 0..2 {
        owning_queue(&f).recover_target("thread-b").await.unwrap();
    }
    assert_eq!(settlement_proof(&f), Some(proof));
    assert_read_only(&f, &[]);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn historical_terminal_never_releases_publishing_policy_or_starts_pending() {
    let f = HistoryFixture::new(&terminal_script(
        "completed",
        Some(user_input(0)),
        json!({"goal":{"threadId":"thread-b","status":"complete"}}),
    ))
    .await;
    cdr_store::schema::open_initialized(&f.db).unwrap().execute(
        "UPDATE cdr_async_execution_obligations SET policy='publishing_recovery' WHERE question_id=?",
        [&f.id],
    ).unwrap();
    let before = queue::list(&f.db).unwrap();
    let report = owning_queue(&f).recover_target("thread-b").await.unwrap();
    assert_eq!(report.started, 0);
    assert_eq!(
        f.obligation(),
        (
            "exact_history_confirmed".into(),
            "terminal".into(),
            "held".into(),
            "original send timeout".into(),
            1
        )
    );
    assert!(cdr_store::async_resolution::admission_held(&f.db, "thread-b").unwrap());
    assert!(cdr_store::async_resolution::guard_mutation(&f.db, "thread-b").is_err());
    assert!(settlement_proof(&f).is_some());
    assert_read_only(&f, &before);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn active_unknown_foreign_goal_or_nonterminal_turn_cannot_settle_execution() {
    let cases = [
        (
            "completed",
            json!({"goal":{"threadId":"thread-b","status":"active"}}),
        ),
        (
            "completed",
            json!({"goal":{"threadId":"thread-b","status":"paused"}}),
        ),
        ("completed", json!({})),
        (
            "completed",
            json!({"goal":{"threadId":"other","status":"complete"}}),
        ),
        ("inProgress", json!({"goal":null})),
    ];
    for (status, goal) in cases {
        let f =
            HistoryFixture::new(&terminal_script(status, Some(user_input(0)), goal.clone())).await;
        queue::complete(&f.db, "next").unwrap();
        owning_queue(&f).recover_target("thread-b").await.unwrap();
        assert_eq!(f.obligation().1, "unresolved", "{status} / {goal}");
        assert!(cdr_store::async_resolution::admission_held(&f.db, "thread-b").unwrap());
        assert!(settlement_proof(&f).is_none());
        assert_read_only(&f, &[]);
        f.server.close().await.unwrap();
    }
}

#[tokio::test]
async fn exact_terminal_without_answer_receipt_does_not_invent_delivery_or_replay() {
    let f = HistoryFixture::new(&terminal_script("failed", None, json!({"goal":null}))).await;
    queue::complete(&f.db, "next").unwrap();
    owning_queue(&f).recover_target("thread-b").await.unwrap();
    assert_eq!(
        f.obligation(),
        (
            "terminal_without_receipt".into(),
            "terminal".into(),
            "settled".into(),
            "original send timeout".into(),
            1
        )
    );
    assert!(settlement_proof(&f).is_some());
    assert!(!cdr_store::async_resolution::admission_held(&f.db, "thread-b").unwrap());
    assert_read_only(&f, &[]);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn certificate_or_question_write_failure_cannot_partially_settle() {
    for mode in ["abort", "ignore", "question"] {
        let f = HistoryFixture::new(&terminal_script(
            "completed",
            Some(user_input(0)),
            json!({"goal":null}),
        ))
        .await;
        queue::complete(&f.db, "next").unwrap();
        let fault = match mode {
            "abort" => {
                "CREATE TRIGGER fail_certificate BEFORE INSERT ON cdr_async_terminal_settlements
                BEGIN SELECT RAISE(ABORT,'injected certificate failure'); END;"
            }
            "ignore" => {
                "CREATE TRIGGER fail_certificate BEFORE INSERT ON cdr_async_terminal_settlements
                BEGIN SELECT RAISE(IGNORE); END;"
            }
            _ => {
                "CREATE TRIGGER fail_close BEFORE UPDATE OF state ON cdr_async_questions
                WHEN NEW.state='closed_unknown' BEGIN SELECT RAISE(IGNORE); END;"
            }
        };
        cdr_store::schema::open_initialized(&f.db)
            .unwrap()
            .execute_batch(fault)
            .unwrap();
        owning_queue(&f).recover_target("thread-b").await.unwrap();
        assert_eq!(
            f.obligation(),
            (
                "exact_history_confirmed".into(),
                "unresolved".into(),
                "held".into(),
                "original send timeout".into(),
                0
            ),
            "{mode}"
        );
        assert_eq!(aq::get(&f.db, &f.id).unwrap().state, "dispatching");
        assert!(settlement_proof(&f).is_none());
        assert_read_only(&f, &[]);
        f.server.close().await.unwrap();
    }
}

#[tokio::test]
async fn mapping_or_policy_change_during_terminal_read_rejects_stale_commit() {
    for mode in ["mapping", "policy"] {
        let mut value = terminal_script("completed", Some(user_input(0)), json!({"goal":null}));
        value["terminal_gate"] = json!(true);
        let f = HistoryFixture::new(&value).await;
        queue::complete(&f.db, "next").unwrap();
        let coordinator = owning_queue(&f);
        let task = tokio::spawn(async move { coordinator.recover_target("thread-b").await });
        wait_file(&f.temp.path().join("rpc.jsonl.terminal-entered")).await;
        let sql = if mode == "mapping" {
            "UPDATE mirror_threads SET discord_thread_id=21 WHERE codex_thread_id='thread-b'"
        } else {
            "UPDATE cdr_async_execution_obligations SET policy='publishing_recovery',revision=revision+1"
        };
        cdr_store::schema::open_initialized(&f.db)
            .unwrap()
            .execute(sql, [])
            .unwrap();
        std::fs::write(f.temp.path().join("rpc.jsonl.terminal-release"), b"release").unwrap();
        task.await.unwrap().unwrap();
        assert_eq!(f.obligation().1, "unresolved", "{mode}");
        assert!(settlement_proof(&f).is_none());
        assert!(cdr_store::async_resolution::admission_held(&f.db, "thread-b").unwrap());
        assert_read_only(&f, &[]);
        f.server.close().await.unwrap();
    }
}

async fn goal_successor_fixture(script: &Value) -> HistoryFixture {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let script_path = temp.path().join("history.json");
    std::fs::write(&script_path, script.to_string()).unwrap();
    let mut config = crate::test_support::native_fixture::config("async-history");
    config.environment.insert(
        "CDR_ASYNC_HISTORY_LOG".into(),
        temp.path().join("rpc.jsonl").to_string_lossy().into(),
    );
    config.environment.insert(
        "CDR_ASYNC_HISTORY_SCRIPT".into(),
        script_path.to_string_lossy().into(),
    );
    let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
    let id = fixture::dispatching(&db, server.instance_id());
    aq::record_error(&db, &id, "original send timeout").unwrap();
    if script["submitted_answer"] == true {
        aq::confirm_dispatch(&db, &id, "original").unwrap();
    }
    cdr_store::async_resolution::record_terminal_notification(
        &db,
        "thread-b",
        "original",
        1,
        server.instance_id(),
        &json!({"threadId":"thread-b","turn":{"id":"original","status":"completed"}}).to_string(),
    )
    .unwrap();
    assert!(queue::mark_goal_waiting(&db, "origin", "original", 1).unwrap());
    let waiting = queue::list(&db).unwrap().remove(0);
    assert!(queue::attach_goal_turn_observed_if_owned(&db, &waiting, "successor", 1).unwrap());
    crate::idle_release::install(&server, &db).unwrap();
    queue::complete(&db, "origin").unwrap();
    let backend = Arc::new(crate::app_backend::AppServerTurnBackend::new(
        server.clone(),
    ));
    HistoryFixture {
        temp,
        db,
        id,
        server,
        backend,
    }
}

#[tokio::test]
async fn historical_goal_settlement_requires_current_verified_successor_not_original_turn() {
    for status in ["inProgress", "missing", "completed"] {
        let mut value = terminal_script(
            "completed",
            Some(user_input(0)),
            json!({"goal":{"threadId":"thread-b","status":"complete"}}),
        );
        if status != "missing" {
            value["pages"][0]["data"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":"successor","status":status,"items":[]}));
        }
        let f = goal_successor_fixture(&value).await;
        owning_queue(&f).recover_target("thread-b").await.unwrap();
        if status == "completed" {
            assert_eq!(f.obligation().1, "terminal");
            assert_eq!(f.obligation().4, 2);
            let proof = settlement_proof(&f).unwrap();
            assert_eq!(proof["turn_id"], "successor");
            assert_eq!(proof["sealed_execution_owner"]["turn_id"], "successor");
            assert!(!cdr_store::async_resolution::admission_held(&f.db, "thread-b").unwrap());
        } else {
            assert_eq!(f.obligation().1, "unresolved", "{status}");
            assert_eq!(f.obligation().4, 1);
            assert!(settlement_proof(&f).is_none());
            assert!(cdr_store::async_resolution::admission_held(&f.db, "thread-b").unwrap());
        }
        assert_read_only(&f, &[]);
        f.server.close().await.unwrap();
    }
}

#[tokio::test]
async fn unknown_or_active_thread_metadata_is_not_execution_completion() {
    for metadata in [Value::Null, json!({"type":"active"}), json!({})] {
        let mut value = terminal_script("completed", Some(user_input(0)), json!({"goal":null}));
        value["thread_status"] = metadata;
        let f = HistoryFixture::new(&value).await;
        queue::complete(&f.db, "next").unwrap();
        owning_queue(&f).recover_target("thread-b").await.unwrap();
        assert_eq!(f.obligation().1, "unresolved");
        assert!(settlement_proof(&f).is_none());
        assert_read_only(&f, &[]);
        f.server.close().await.unwrap();
    }
}

#[tokio::test]
async fn cancelled_terminal_read_preserves_hold_and_cold_review_uses_fresh_reads() {
    let mut value = terminal_script("completed", Some(user_input(0)), json!({"goal":null}));
    value["terminal_gate"] = json!(true);
    let f = HistoryFixture::new(&value).await;
    queue::complete(&f.db, "next").unwrap();
    let coordinator = owning_queue(&f);
    let task = tokio::spawn(async move { coordinator.recover_target("thread-b").await });
    wait_file(&f.temp.path().join("rpc.jsonl.terminal-entered")).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(f.obligation().1, "unresolved");
    assert!(settlement_proof(&f).is_none());
    let before = f.calls().len();
    std::fs::write(
        f.temp.path().join("rpc.jsonl.terminal-release"),
        b"release cancelled read",
    )
    .unwrap();
    owning_queue(&f).recover_target("thread-b").await.unwrap();
    assert!(
        f.calls().len() > before,
        "cold review reused cancelled in-memory authority"
    );
    assert_eq!(f.obligation().1, "terminal");
    assert!(settlement_proof(&f).is_some());
    assert_read_only(&f, &[]);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn lost_goal_response_keeps_original_held_and_same_resident_other_read_healthy() {
    let mut value = terminal_script("completed", Some(user_input(0)), json!({"goal":null}));
    value["terminal_timeout"] = json!(true);
    let f = HistoryFixture::new(&value).await;
    queue::complete(&f.db, "next").unwrap();
    owning_queue(&f).recover_target("thread-b").await.unwrap();
    assert_eq!(f.obligation().1, "unresolved");
    assert!(settlement_proof(&f).is_none());
    let other = f
        .server
        .execute(
            cdr_app_server::requests::read_thread("healthy-b", false),
            Some(1),
        )
        .await
        .unwrap();
    assert_eq!(other["thread"]["id"], "healthy-b");
    assert_read_only(&f, &[]);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn after_close_side_effects_cannot_commit_reopened_or_changed_source() {
    for mode in ["reopen", "choice", "seal", "error"] {
        let f = HistoryFixture::new(&terminal_script(
            "completed",
            Some(user_input(0)),
            json!({"goal":null}),
        ))
        .await;
        queue::complete(&f.db, "next").unwrap();
        let assignment = match mode {
            "reopen" => "state='open'",
            "choice" => "chosen=1",
            "seal" => "preparation_json='{}'",
            _ => "error='overwritten after close'",
        };
        cdr_store::schema::open_initialized(&f.db)
            .unwrap()
            .execute_batch(&format!(
                "CREATE TRIGGER change_after_historical_close
             AFTER UPDATE OF state ON cdr_async_questions WHEN NEW.state='closed_unknown'
             BEGIN UPDATE cdr_async_questions SET {assignment} WHERE id=NEW.id; END;"
            ))
            .unwrap();
        let original = aq::get(&f.db, &f.id).unwrap();
        owning_queue(&f).recover_target("thread-b").await.unwrap();
        assert_eq!(
            f.obligation(),
            (
                "exact_history_confirmed".into(),
                "unresolved".into(),
                "held".into(),
                "original send timeout".into(),
                0
            ),
            "{mode}"
        );
        let after = aq::get(&f.db, &f.id).unwrap();
        assert_eq!(after.state, "dispatching", "{mode}");
        assert_eq!(after.chosen, original.chosen, "{mode}");
        assert_eq!(after.error, original.error, "{mode}");
        assert!(settlement_proof(&f).is_none(), "{mode}");
        assert_read_only(&f, &[]);
        f.server.close().await.unwrap();
    }
}

#[tokio::test]
async fn submitted_source_keeps_its_state_identity_and_error_after_terminal_commit() {
    let mut value = terminal_script(
        "completed",
        Some(user_input(0)),
        json!({"goal":{"threadId":"thread-b","status":"complete"}}),
    );
    value["submitted_answer"] = json!(true);
    value["pages"][0]["data"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"successor","status":"completed","items":[]}));
    let f = goal_successor_fixture(&value).await;
    let before = aq::get(&f.db, &f.id).unwrap();
    assert_eq!(before.state, "submitted");
    owning_queue(&f).recover_target("thread-b").await.unwrap();
    let after = aq::get(&f.db, &f.id).unwrap();
    assert_eq!(after.state, "submitted");
    assert_eq!(after.error, before.error);
    assert_eq!(after.chosen, before.chosen);
    assert_eq!(after.origin_job_id, before.origin_job_id);
    assert_eq!(f.obligation().1, "terminal");
    assert_eq!(f.obligation().4, 2);
    assert!(settlement_proof(&f).is_some());
    assert!(queue::list(&f.db).unwrap().is_empty());
    assert!(f.calls().iter().all(|v| matches!(
        v["method"].as_str(),
        Some("initialize" | "thread/read" | "thread/turns/list" | "thread/goal/get")
    )));
    f.server.close().await.unwrap();
}
