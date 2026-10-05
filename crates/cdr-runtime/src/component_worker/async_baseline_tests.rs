//! One-shot answers retain the pre-start boundary used by Goal completion.
use super::*;

async fn history_kind(f: &MessageFixture, kind: &str) {
    f.server
        .execute(
            AppRequest {
                method: "test/history-kind",
                params: json!({"kind":kind}),
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
async fn invalid_or_foreign_start_history_never_claims_or_sends_an_answer() {
    for kind in [
        "wrong-thread",
        "duplicate",
        "missing-original",
        "active",
        "missing-turns",
    ] {
        let http = approval_http::start().await;
        let temp = tempfile::tempdir().unwrap();
        let f = fixture(&temp, http_client(&http.address)).await;
        let (id, work) = question(&f).await;
        queue::complete(f.executor.mirror_db(), "origin").unwrap();
        history_kind(&f, kind).await;
        assert!(!click(&f, &id, &work).await, "{kind}");
        assert!(queue::list(f.executor.mirror_db()).unwrap().is_empty());
        assert_eq!(aq::get(f.executor.mirror_db(), &id).unwrap().state, "open");
        f.server.close().await.unwrap();
        http.stop.send(()).unwrap();
        http.task.await.unwrap();
        assert!(
            !app_fixture::rpc_log(&temp.path().join("rpc.jsonl"))
                .iter()
                .any(|v| v["method"] == "turn/start"),
            "{kind}"
        );
    }
}

async fn completion_case(legacy_empty_baseline: bool, unobserved_successor: bool) {
    let http = approval_http::start().await;
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp, http_client(&http.address)).await;
    let (id, work) = question(&f).await;
    let db = f.executor.mirror_db();
    queue::complete(db, "origin").unwrap();
    history_kind(&f, "prior").await;
    control(&f, "test/goal-complete").await;
    assert!(click(&f, &id, &work).await);
    assert!(
        click(&f, &id, &work).await,
        "duplicate is confirmation only"
    );
    let before = queue::list(db).unwrap().remove(0);
    assert_eq!(before.baseline_turn_ids, ["older-ui-turn", "original"]);
    if legacy_empty_baseline {
        // Negative control: reproduce the deployed pre-fix durable encoding.
        cdr_store::schema::open_initialized(db)
            .unwrap()
            .execute(
                "UPDATE codex_turn_queue SET baseline_turn_ids='[]' WHERE job_id=?",
                [&before.job_id],
            )
            .unwrap();
    }
    if unobserved_successor {
        history_kind(&f, "successor").await;
    }
    let held = legacy_empty_baseline || unobserved_successor;
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(crate::completion_worker::run_completion_worker(
        f.server.subscribe_notifications(),
        Arc::clone(&f.server),
        Arc::clone(&f.queue),
        Arc::clone(&f.http),
        false,
        Duration::from_secs(2),
        shutdown,
    ));
    control(&f, "test/complete-answer").await;
    let observed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let jobs = queue::list(db).unwrap();
            let settled = if held {
                jobs.len() == 1
                    && jobs[0].goal_waiting
                    && cdr_store::goal_progress::pending(db).unwrap().is_empty()
            } else {
                jobs.is_empty() && cdr_store::delivery::list_pending(db).unwrap().is_empty()
            };
            if settled {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(3), worker)
        .await
        .unwrap()
        .unwrap();
    assert!(
        observed.is_ok(),
        "legacy={legacy_empty_baseline}, successor={unobserved_successor}"
    );
    let jobs = queue::list(db).unwrap();
    if held {
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_id, before.job_id);
        assert_eq!(jobs[0].turn_id, before.turn_id);
        assert_eq!(jobs[0].prompt, before.prompt);
        assert_eq!(jobs[0].app_server_generation, before.app_server_generation);
        assert_eq!(jobs[0].attempt_count, before.attempt_count);
        assert!(jobs[0].goal_waiting);
    } else {
        assert!(
            jobs.is_empty(),
            "completed answer must not strand controls or mirroring"
        );
    }
    f.server.close().await.unwrap();
    http.stop.send(()).unwrap();
    let traffic = http.task.await.unwrap();
    let progress = traffic.iter().any(|(_, body)| {
        body["content"]
            .as_str()
            .is_some_and(|text| text.contains("[Goal progress]"))
    });
    assert_eq!(progress, held);
    let rpc = app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
    assert_eq!(
        rpc.iter().filter(|v| v["method"] == "turn/start").count(),
        1
    );
    assert!(!rpc.iter().any(|v| v["method"] == "thread/settings/update"));
}

#[tokio::test]
async fn saved_async_baseline_finishes_terminal_goal_without_false_waiting() {
    completion_case(false, false).await;
}

#[tokio::test]
async fn legacy_empty_async_baseline_reproduces_false_goal_waiting() {
    completion_case(true, false).await;
}

#[tokio::test]
async fn real_unobserved_successor_is_not_adopted_or_replayed() {
    completion_case(false, true).await;
}
