use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    command_plan::CommandAction, queue_runner::QueueCoordinator,
};
use std::{sync::Arc, time::Duration};
#[path = "support/archive_app_server.rs"]
mod support;

#[tokio::test]
async fn inner_archive_timeout_and_disconnect_report_uncertainty_but_writer_rejection_is_distinct()
{
    for scenario in [
        "stall_archive_inner",
        "archive_disconnect",
        "archive_writer_reject",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state.sqlite");
        rusqlite::Connection::open(&state)
            .unwrap()
            .execute_batch(include_str!("fixtures/action_state.sql"))
            .unwrap();
        let server = Arc::new(support::start(&temp, &state, scenario).await);
        let db = temp.path().join("mirror.sqlite");
        let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
        bridge.set_selected_thread_id(Some("thread-b")).unwrap();
        let executor = ActionExecutor::new(
            state.clone(),
            db.clone(),
            bridge.clone(),
            Arc::new(QueueCoordinator::new(
                db.clone(),
                Arc::new(AppServerTurnBackend::new(server.clone())),
            )),
        )
        .with_server(server.clone()); // Default outer deadline outlives the inner 10s RPC deadline.
        let observed = tokio::time::timeout(
            Duration::from_secs(15),
            executor.execute(CommandAction::Archive { reference: None }, 99, 20),
        )
        .await;
        server.close().await.unwrap();
        let error = observed.unwrap().unwrap_err().to_string();
        let fences: i64 = rusqlite::Connection::open(&db)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM codex_archive_fences WHERE phase='attempted'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(fences, i64::from(scenario != "archive_writer_reject"));
        if scenario == "archive_writer_reject" {
            assert!(error.contains("already has an active writer"), "{error}");
            assert!(!error.contains("may already be archived"), "{error}");
        } else {
            assert!(error.contains("may already be archived"), "{error}");
            assert!(error.contains("do not automatically retry"), "{error}");
            assert!(
                error.contains("thread/archive"),
                "underlying error lost: {error}"
            );
        }
        assert_eq!(
            bridge.selected_thread_id().unwrap().as_deref(),
            Some("thread-b")
        );
        let calls = support::calls(&temp);
        assert_eq!(
            calls
                .iter()
                .filter(|v| v["method"] == "thread/archive")
                .count(),
            1
        );
        assert!(
            !calls
                .iter()
                .any(|v| matches!(v["method"].as_str(), Some("thread/fork" | "turn/start")))
        );
        assert_eq!(
            cdr_codex_state::CodexThreadStore::open(&state)
                .unwrap()
                .load_thread("thread-b", true)
                .unwrap()
                .is_some(),
            scenario != "archive_writer_reject"
        );
    }
}
