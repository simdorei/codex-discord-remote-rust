use super::*;
use crate::test_support::{message_fixture::MessageFixture, native_fixture};

#[path = "repair_custody_tests.rs"]
mod custody;

async fn fixture(temp: &tempfile::TempDir, mode: &str) -> MessageFixture {
    let mut config = native_fixture::config("repair");
    config.environment.insert(
        "CDR_REPAIR_RPC_LOG".into(),
        temp.path().join("rpc.jsonl").to_string_lossy().into(),
    );
    config
        .environment
        .insert("CDR_REPAIR_MODE".into(), mode.into());
    let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
    MessageFixture::with_server(
        temp,
        Arc::new(twilight_http::Client::new("fixture".into())),
        server,
    )
}

fn calls(temp: &tempfile::TempDir) -> Vec<Value> {
    std::fs::read_to_string(temp.path().join("rpc.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn repair_resets_only_a_target_and_probes_without_a_model_turn_or_restart() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = fixture(&temp, "ready").await;
    let before = fixture.server.generation();
    let result = fixture.executor.repair_tools(42, None).await.unwrap();
    assert!(result.text.contains("도구 복구 완료"));
    let calls = calls(&temp);
    let methods: Vec<_> = calls
        .iter()
        .map(|c| c["method"].as_str().unwrap())
        .collect();
    assert_eq!(
        methods,
        [
            "initialize",
            "thread/read",
            "mcpServerStatus/list",
            "mcpServer/tool/call",
            "mcpServer/tool/call",
            "mcpServer/tool/call"
        ]
    );
    for call in calls.iter().skip(1) {
        assert_eq!(call["params"]["threadId"], "thread-b");
    }
    assert_eq!(calls[3]["params"]["tool"], "js_reset");
    assert_eq!(calls[4]["params"]["arguments"]["code"], INIT);
    assert_eq!(calls[5]["params"]["arguments"]["code"], PROBE);
    assert_eq!(fixture.server.generation(), before);
    assert!(
        fixture
            .queue
            .target_lock("thread-b")
            .unwrap()
            .try_lock_owned()
            .is_ok()
    );
    fixture.server.close().await.unwrap();
}

#[tokio::test]
async fn busy_or_unsupported_target_is_never_reset() {
    for mode in ["busy", "missing"] {
        let temp = tempfile::tempdir().unwrap();
        let fixture = fixture(&temp, mode).await;
        assert!(fixture.executor.repair_tools(42, None).await.is_err());
        assert!(
            !calls(&temp)
                .iter()
                .any(|c| c["method"] == "mcpServer/tool/call")
        );
        fixture.server.close().await.unwrap();
    }
}

#[tokio::test]
async fn pipe_failure_does_not_report_success_or_restart() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = fixture(&temp, "pipe").await;
    let error = fixture
        .executor
        .repair_tools(42, None)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("native pipe"));
    assert!(!error.contains("도구 복구 완료"));
    assert_eq!(fixture.server.generation(), 1);
    assert!(
        fixture
            .queue
            .target_lock("thread-b")
            .unwrap()
            .try_lock_owned()
            .is_ok()
    );
    fixture.server.close().await.unwrap();
}

#[tokio::test]
async fn unknown_reset_holds_only_a_until_backend_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = fixture(&temp, "timeout").await;
    let error = fixture
        .executor
        .repair_tools(42, None)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("!recover"));
    assert!(
        fixture
            .queue
            .target_lock("thread-b")
            .unwrap()
            .try_lock_owned()
            .is_err()
    );
    assert!(
        fixture
            .queue
            .target_lock("thread-a")
            .unwrap()
            .try_lock_owned()
            .is_ok()
    );
    assert!(!fixture.server.lifecycle_snapshot().await.quarantined);
    let other = fixture
        .server
        .request("thread/read", json!({"threadId":"thread-a"}), WAIT, Some(1))
        .await
        .unwrap();
    assert_eq!(other["thread"]["status"]["type"], "idle");
    assert_eq!(
        calls(&temp)
            .iter()
            .filter(|c| c["params"]["tool"] == "js_reset")
            .count(),
        1
    );
    fixture.server.close().await.unwrap();
}

#[tokio::test]
async fn a_stuck_target_lock_does_not_make_repair_wait_forever() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = fixture(&temp, "ready").await;
    let _lock = fixture
        .queue
        .target_lock("thread-b")
        .unwrap()
        .lock_owned()
        .await;
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        fixture.executor.repair_tools(42, None),
    )
    .await
    .unwrap();
    assert!(result.unwrap_err().to_string().contains("!recover"));
    assert_eq!(calls(&temp).len(), 1);
    fixture.server.close().await.unwrap();
}

#[tokio::test]
async fn repair_preserves_queued_requests_without_sending_a_reset() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = fixture(&temp, "ready").await;
    let db = fixture.executor.mirror_db();
    cdr_store::queue::enqueue(
        db,
        cdr_store::queue::NewQueueJob {
            job_id: "waiting",
            target_thread_id: "thread-b",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: Some(5),
            app_server_generation: 1,
            prompt: "preserve request",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    let before = cdr_store::queue::list(db).unwrap();
    assert!(fixture.executor.repair_tools(42, None).await.is_err());
    assert_eq!(cdr_store::queue::list(db).unwrap(), before);
    assert_eq!(calls(&temp).len(), 1);
    fixture.server.close().await.unwrap();
}
