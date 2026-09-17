use super::*;
use cdr_store::reserve_policy;
use serde_json::Value;

async fn configured(temp: &tempfile::TempDir, http: Arc<Client>) -> MessageFixture {
    let mut config = native_fixture::config("reserve-auto");
    config.environment.insert(
        "RESERVE_TEST_LOG".into(),
        temp.path().join("rpc.jsonl").to_string_lossy().into_owned(),
    );
    let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
    MessageFixture::with_reserve_server(temp, http, server)
}
async fn configure(f: &MessageFixture, params: Value) {
    f.server
        .execute(
            AppRequest {
                method: "test/configure",
                params,
                timeout: Duration::from_secs(2),
            },
            Some(1),
        )
        .await
        .unwrap();
}
async fn click(f: &MessageFixture, id: &str, work: &InboundInteractionWork) -> bool {
    handle_component_work(
        work,
        &ComponentId::AsyncChoice {
            question_id: id.into(),
            option: 1,
        },
        &f.executor,
        &f.server,
    )
    .await
    .is_ok()
}
#[tokio::test]
async fn actual_async_start_uses_reserve_effort_policy_for_true_null_and_missing_ordinary() {
    for availability in [json!(true), Value::Null, json!("omit")] {
        let http = approval_http::start().await;
        let temp = tempfile::tempdir().unwrap();
        let f = configured(&temp, http_client(&http.address)).await;
        configure(&f,json!({"ordinary":availability,"omit_ordinary":availability==json!("omit"),"settings":{"model":"gpt-reserve","effort":"high","serviceTier":"default"}})).await;
        let (id, work) = question(&f).await;
        queue::complete(f.executor.mirror_db(), "origin").unwrap();
        assert!(click(&f, &id, &work).await, "{availability}");
        assert!(click(&f, &id, &work).await, "confirmation only");
        let jobs = queue::list(f.executor.mirror_db()).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].state, queue::QueueJobState::Running);
        let rpc = app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
        assert_eq!(
            rpc.iter().filter(|v| v["method"] == "turn/start").count(),
            1
        );
        let start = rpc.iter().find(|v| v["event"] == "start_settings").unwrap();
        assert_eq!(start["settings"]["model"], "gpt-reserve");
        assert_eq!(start["settings"]["effort"], "medium");
        assert!(rpc.iter().any(|v| v["method"] == "thread/settings/update"));
        f.server.close().await.unwrap();
        http.stop.send(()).unwrap();
        http.task.await.unwrap();
    }
}

#[tokio::test]
async fn unsupported_reserve_effort_does_not_claim_or_start_async_answer() {
    let http = approval_http::start().await;
    let temp = tempfile::tempdir().unwrap();
    let f = configured(&temp, http_client(&http.address)).await;
    configure(
        &f,
        json!({"ordinary":false,"reserve_efforts":[{"reasoningEffort":"low"}]}),
    )
    .await;
    let (id, work) = question(&f).await;
    queue::complete(f.executor.mirror_db(), "origin").unwrap();
    assert!(!click(&f, &id, &work).await);
    assert_eq!(aq::get(f.executor.mirror_db(), &id).unwrap().state, "open");
    assert!(queue::list(f.executor.mirror_db()).unwrap().is_empty());
    let rpc = app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
    assert!(
        !rpc.iter()
            .any(|v| v["method"] == "turn/start" || v["method"] == "thread/settings/update")
    );
    f.server.close().await.unwrap();
    http.stop.send(()).unwrap();
    http.task.await.unwrap();
}

#[tokio::test]
async fn active_question_steer_never_mutates_reserve_settings() {
    let http = approval_http::start().await;
    let temp = tempfile::tempdir().unwrap();
    let f = configured(&temp, http_client(&http.address)).await;
    configure(
        &f,
        json!({"ordinary":false,"reserve_efforts":[{"reasoningEffort":"low"}]}),
    )
    .await;
    control(&f, "test/active-turn").await;
    let (id, work) = question(&f).await;
    assert!(click(&f, &id, &work).await);
    let rpc = app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
    assert_eq!(
        rpc.iter().filter(|v| v["method"] == "turn/steer").count(),
        1
    );
    assert!(!rpc.iter().any(|v| matches!(
        v["method"].as_str(),
        Some("turn/start" | "thread/resume" | "thread/settings/update" | "account/rateLimits/read")
    )));
    f.server.close().await.unwrap();
    http.stop.send(()).unwrap();
    http.task.await.unwrap();
}

#[tokio::test]
async fn real_resident_mutation_guard_rejects_post_claim_policy_change_without_start() {
    let http = approval_http::start().await;
    let temp = tempfile::tempdir().unwrap();
    let f = configured(&temp, http_client(&http.address)).await;
    let (id, _work) = question(&f).await;
    let db = f.executor.mirror_db();
    queue::complete(db, "origin").unwrap();
    reserve_policy::ensure(db, "thread-b").unwrap();
    let q = aq::get(db, &id).unwrap();
    aq::begin_dispatch(
        db,
        &aq::Claim {
            id: &id,
            runtime_id: f.server.instance_id(),
            generation: 1,
            channel: 42,
            actor: 3,
            message: q.message_id.as_deref().unwrap(),
            option: 1,
            mode: aq::DispatchMode::Start,
            prompt: "exact answer",
            now: 2.0,
        },
    )
    .unwrap();
    reserve_policy::stage_usage_failure(db, "thread-b", "after claim").unwrap();
    let error = f
        .server
        .execute(
            cdr_app_server::requests::start_turn("thread-b", "exact answer"),
            Some(1),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Reserve"), "{error}");
    assert!(
        !app_fixture::rpc_log(&temp.path().join("rpc.jsonl"))
            .iter()
            .any(|v| v["method"] == "turn/start")
    );
    assert_eq!(aq::get(db, &id).unwrap().state, "dispatching");
    f.server.close().await.unwrap();
    http.stop.send(()).unwrap();
    http.task.await.unwrap();
}

#[tokio::test]
async fn typed_async_start_failure_is_not_replayed_after_reserve_switch() {
    let http = approval_http::start().await;
    let temp = tempfile::tempdir().unwrap();
    let f = configured(&temp, http_client(&http.address)).await;
    configure(&f, json!({"ordinary":true,"reject_next_start":"usage"})).await;
    let (id, work) = question(&f).await;
    let db = f.executor.mirror_db();
    queue::complete(db, "origin").unwrap();
    assert!(!click(&f, &id, &work).await);
    assert!(!click(&f, &id, &work).await);
    assert_eq!(aq::get(db, &id).unwrap().state, "rejected");
    assert!(queue::list(db).unwrap().is_empty());
    assert_eq!(
        reserve_policy::get(db, "thread-b").unwrap().unwrap().state,
        "reserve"
    );
    assert!(!reserve_policy::usage_failure_unresolved(db, "thread-b").unwrap());
    assert_eq!(
        app_fixture::rpc_log(&temp.path().join("rpc.jsonl"))
            .iter()
            .filter(|v| v["method"] == "turn/start")
            .count(),
        1
    );
    f.server.close().await.unwrap();
    http.stop.send(()).unwrap();
    http.task.await.unwrap();
}

#[tokio::test]
async fn failed_question_claim_after_successful_preparation_never_starts_answer() {
    let http = approval_http::start().await;
    let temp = tempfile::tempdir().unwrap();
    let f = configured(&temp, http_client(&http.address)).await;
    let (id, work) = question(&f).await;
    let db = f.executor.mirror_db();
    queue::complete(db, "origin").unwrap();
    cdr_store::schema::open_initialized(db).unwrap().execute_batch("CREATE TRIGGER fail_question_claim BEFORE UPDATE ON cdr_async_questions WHEN NEW.state='dispatching' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(!click(&f, &id, &work).await);
    assert_eq!(aq::get(db, &id).unwrap().state, "open");
    assert!(queue::list(db).unwrap().is_empty());
    assert!(
        !app_fixture::rpc_log(&temp.path().join("rpc.jsonl"))
            .iter()
            .any(|v| v["method"] == "turn/start")
    );
    f.server.close().await.unwrap();
    http.stop.send(()).unwrap();
    http.task.await.unwrap();
}
