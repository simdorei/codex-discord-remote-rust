use cdr_runtime::server_prompt_redisplay;
#[path = "support/approval_app_server.rs"]
mod app;

#[tokio::test]
async fn closed_empty_request_cache_is_not_reported_as_no_pending_approvals() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    let server = app::start(&temp, &temp.path().join("rpc.jsonl")).await;
    assert!(
        server_prompt_redisplay::prepare(&db, &server, "thread-b", 42, 3)
            .await
            .unwrap()
            .is_empty()
    );
    server.close().await.unwrap();
    assert!(
        server_prompt_redisplay::prepare(&db, &server, "thread-b", 42, 3)
            .await
            .is_err()
    );
}
