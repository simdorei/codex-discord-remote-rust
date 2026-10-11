#[allow(dead_code)]
#[path = "../../tests/support/mutation_isolation.rs"]
mod support;

use super::*;
use crate::dead_generation_recovery::RuntimeDeadGenerationFence;
use crate::queue_runner::TurnBackend;
use std::sync::Arc;

async fn start(db: &Path, log: &Path, runtime: &str, mode: &str) -> Arc<ResidentAppServer> {
    let mut config = support::config(log, mode);
    config.arguments[1] =
        "discord_runtime::resume_recovery::tests::support::p03b_fixture_child".into();
    let fence = Arc::new(RuntimeDeadGenerationFence::new(db.into(), runtime.into(), None).unwrap());
    Arc::new(
        ResidentAppServer::start_with_dead_generation_fence(config, fence)
            .await
            .unwrap(),
    )
}

#[tokio::test]
async fn cold_restart_recovers_idle_resume_without_replaying_a_prompt() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let first = start(&db, &log, "old", "normal").await;
    let result = first
        .request(
            "thread/resume",
            json!({"threadId":"A","fixture":"hold"}),
            support::EXPIRY,
            Some(1),
        )
        .await;
    assert!(result.is_err());
    recover(&first, &db, "old").await;
    assert!(cdr_store::mutation_attempt::check(&db, "old", Some("A")).is_err());
    first.close().await.unwrap();
    let next = start(&db, &log, "new", "normal").await;
    recover(&next, &db, "new").await;
    cdr_store::mutation_attempt::check(&db, "new", Some("A")).unwrap();
    crate::app_backend::AppServerTurnBackend::new(next.clone())
        .resume_thread("A")
        .await
        .unwrap();
    next.close().await.unwrap();
    assert_eq!(support::count(&log, "thread/resume", "A"), 2);
    assert_eq!(support::count(&log, "turn/start", "A"), 0);
    let calls = support::calls(&log);
    let resumed = calls
        .iter()
        .rfind(|c| c["method"] == "thread/resume")
        .unwrap();
    assert_eq!(resumed["params"]["excludeTurns"], true);
}

#[tokio::test]
async fn cold_restart_holds_active_unreadable_or_malformed_threads() {
    for mode in [
        "active-resume-recovery",
        "error-resume-recovery",
        "malformed-resume-recovery",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        let log = temp.path().join("rpc.jsonl");
        let first = start(&db, &log, "old", "normal").await;
        let _ = first
            .request(
                "thread/resume",
                json!({"threadId":"A","fixture":"hold"}),
                support::EXPIRY,
                Some(1),
            )
            .await;
        first.close().await.unwrap();
        let next = start(&db, &log, "new", mode).await;
        recover(&next, &db, "new").await;
        assert!(cdr_store::mutation_attempt::check(&db, "new", Some("A")).is_err());
        next.close().await.unwrap();
        assert_eq!(support::count(&log, "thread/resume", "A"), 1);
        assert_eq!(support::count(&log, "turn/start", "A"), 0);
    }
}
