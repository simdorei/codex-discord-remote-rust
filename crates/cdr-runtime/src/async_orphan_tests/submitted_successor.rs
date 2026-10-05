use cdr_app_server::{AppServerError, ResidentAppServer, requests::AppRequest};
use serde_json::json;
use std::{sync::Arc, time::Duration};

use super::submitted_fixture as fixture;

#[tokio::test]
async fn ack_first_successor_archive_fence_blocks_actual_journal_transport() {
    for phase in [None, Some("attempted"), Some("verified")] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("state.sqlite");
        let rpc = temp.path().join("rpc.jsonl");
        let mut config = crate::test_support::native_fixture::config("async-question");
        config.environment.insert(
            "CDR_ACTION_RPC_LOG".into(),
            rpc.to_string_lossy().into_owned(),
        );
        let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
        let id = fixture::successor(&db, server.instance_id());
        fixture::pending(&db, "next", "thread-b", 2);
        crate::idle_release::install(&server, &db).unwrap();
        if let Some(phase) = phase {
            cdr_store::schema::open_initialized(&db).unwrap().execute(
                "INSERT INTO codex_archive_fences(target_thread_id,operation_id,phase) VALUES('thread-b','archive',?)",
                [phase],
            ).unwrap();
        }
        let before = fixture::snapshot(&db);
        let result = server
            .execute(
                AppRequest {
                    method: "thread/resume",
                    params: json!({"threadId": "thread-b"}),
                    timeout: Duration::from_secs(2),
                },
                Some(1),
            )
            .await;
        server.close().await.unwrap();
        let trace = std::fs::read_to_string(&rpc).unwrap_or_default();
        let frames = trace
            .lines()
            .filter(|line| line.contains("thread/resume"))
            .count();
        assert_eq!(fixture::snapshot(&db), before);
        assert_eq!(
            cdr_store::async_question::get(&db, &id).unwrap().state,
            "submitted"
        );
        if phase.is_some() {
            assert_eq!(
                frames, 0,
                "ACK-first fenced successor reached transport: {trace}"
            );
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("mapping changed or target is fenced")
            );
        } else {
            assert_eq!(
                frames, 1,
                "positive control must reach native transport: {trace}"
            );
            assert!(
                matches!(result, Err(AppServerError::Remote { code: -32601, .. })),
                "offline fixture does not implement resume; expect its explicit remote rejection"
            );
        }
    }
}
