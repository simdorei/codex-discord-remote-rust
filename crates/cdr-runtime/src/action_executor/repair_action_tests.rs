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
async fn unified_repair_confirms_reset_without_invoking_cua_out_of_turn() {
    for mode in [
        "unified_turn_required",
        "unified_ready",
        "unified_both",
        "unified_paged",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let fixture = fixture(&temp, mode).await;
        let result = fixture.executor.repair_tools(42, None).await.unwrap();
        assert!(result.text.contains("cua_repl"));
        assert!(result.text.contains("세션 초기화 완료"));
        assert!(result.text.contains("아직 확인하지 않았습니다"));
        assert!(!result.text.contains("도구 복구 완료"));
        let calls = calls(&temp);
        let tools: Vec<_> = calls
            .iter()
            .filter(|c| c["method"] == "mcpServer/tool/call")
            .collect();
        assert_eq!(tools.len(), 1);
        for call in &tools {
            assert_eq!(call["params"]["server"], "cua_repl");
            assert_eq!(call["params"]["threadId"], "thread-b");
        }
        assert_eq!(tools[0]["params"]["tool"], "js_reset");
        assert!(!calls.iter().any(|call| call["method"] == "turn/start"));
        let receipt = std::fs::read_dir(temp.path().join("maintenance_backups/tool-repair"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let saved: Value = serde_json::from_slice(&std::fs::read(receipt).unwrap()).unwrap();
        assert_eq!(saved["phase"], "reset_confirmed");
        assert_eq!(saved["verification_scope"], "js_session_reset_only");
        for field in [
            "native_desktop_state",
            "browser_state",
            "desktop_app_independence",
        ] {
            assert_eq!(saved[field], "NOT_CHECKED");
        }
        assert_eq!(fixture.server.generation(), 1);
        fixture.server.close().await.unwrap();
    }
}

#[tokio::test]
async fn unified_repair_requires_a_confirmed_reset_before_reporting_completion() {
    for mode in ["unified_reset_error", "unified_reset_malformed"] {
        let temp = tempfile::tempdir().unwrap();
        let fixture = fixture(&temp, mode).await;
        let error = fixture.executor.repair_tools(42, None).await.unwrap_err();
        assert!(!error.to_string().contains("세션 초기화 완료"));
        let tools: Vec<_> = calls(&temp)
            .into_iter()
            .filter(|call| call["method"] == "mcpServer/tool/call")
            .collect();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["params"]["tool"], "js_reset");
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
