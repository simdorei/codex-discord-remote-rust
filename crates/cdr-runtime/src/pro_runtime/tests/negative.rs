use super::*;
use std::time::Duration;

#[tokio::test]
async fn actual_pro_failures_never_start_a_turn_or_choose_another_project() {
    for (case, expected) in [
        ("offline", "connection did not become ready"),
        ("stale", "plugins changed"),
        ("missing", "project directory could not be verified"),
        ("invalid-plugin", "invalid plugin source metadata"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let gate = http_gate::start().await;
        let http = Arc::new(
            twilight_http::Client::builder()
                .proxy(gate.address, true)
                .ratelimiter(None)
                .build(),
        );
        gate.release.send(()).unwrap();
        let mut fixture = MessageFixture::new(&temp, http).await;
        let project = temp.path().join("original-project");
        std::fs::create_dir(&project).unwrap();
        rusqlite::Connection::open(temp.path().join("state.sqlite"))
            .unwrap()
            .execute(
                "UPDATE threads SET cwd=?1 WHERE id='thread-b'",
                [project.to_str().unwrap()],
            )
            .unwrap();
        let mut connected = Some(connection::Connection::start().await);
        let connection = connected.as_ref().unwrap();
        let pro = ProPromptRuntime::capture(
            temp.path().to_owned(),
            temp.path().join("state.sqlite"),
            plugins::installed(temp.path()),
            fixture.server.clone(),
            Some(connection.config.clone()),
            connection.status.clone(),
        )
        .await;
        match case {
            "offline" => connected.take().unwrap().close().await,
            "stale" => {
                // Another owned turn makes automatic quiescent refresh unavailable.
                fixture
                    .server
                    .request(
                        "test/active-turn",
                        serde_json::json!({"threadId":"other-thread","turnId":"other-turn"}),
                        Duration::from_secs(2),
                        None,
                    )
                    .await
                    .unwrap();
                std::fs::write(temp.path().join("chrome/plugin.txt"), "changed fixture").unwrap();
            }
            "missing" => std::fs::remove_dir(&project).unwrap(),
            "invalid-plugin" => {
                std::fs::write(temp.path().join("inventory.json"), "not JSON").unwrap();
            }
            _ => unreachable!(),
        }
        fixture.executor = fixture.executor.with_prompt_preprocessor(Arc::new(pro));
        let generation = fixture.server.generation();
        let admitted = fixture.admit("!pro review 확인");
        let result = crate::message_worker::process_admitted_gateway_message(
            admitted,
            &fixture.context(temp.path()),
        )
        .await;
        let error = result.unwrap_err().to_string();
        gate.stop.send(()).unwrap();
        assert!(gate.task.await.unwrap().is_empty());
        assert!(error.contains(expected), "{case}: {error}");
        assert_eq!(fixture.server.generation(), generation);
        if let Some(connection) = connected {
            connection.close().await;
        }
        fixture.server.close().await.unwrap();
        let calls = crate::test_support::app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
        assert!(!calls.iter().any(|v| matches!(
            v["method"].as_str(),
            Some("turn/start" | "thread/fork" | "thread/start")
        )));
        let ingress = cdr_store::ingress::get(fixture.executor.mirror_db(), "message:801")
            .unwrap()
            .unwrap();
        assert_eq!(ingress.target_thread_id.as_deref(), Some("thread-b"));
    }
}

#[tokio::test]
async fn ordinary_text_does_not_require_pro_plugins_or_remote_connection() {
    let temp = tempfile::tempdir().unwrap();
    let server = Arc::new(
        crate::test_support::app_fixture::start_fake_server(&temp, &temp.path().join("rpc.jsonl"))
            .await,
    );
    let pro = ProPromptRuntime::capture(
        temp.path().to_owned(),
        temp.path().join("absent.sqlite"),
        temp.path().join("missing-cli.exe"),
        server.clone(),
        None,
        RemoteAgentStatus::default(),
    )
    .await;
    assert_eq!(
        pro.prepare("ordinary request", "original").await.unwrap(),
        "ordinary request"
    );
    server.close().await.unwrap();
}

#[tokio::test]
async fn external_ownership_mode_ignores_embedded_status_and_requires_verified_external_status() {
    let temp = tempfile::tempdir().unwrap();
    let server = Arc::new(
        crate::test_support::app_fixture::start_fake_server(&temp, &temp.path().join("rpc.jsonl"))
            .await,
    );
    let connected = connection::Connection::start().await;
    let mut external_config = connected.config.clone();
    external_config.ownership = cdr_remote_agent::config::RemoteMcpOwnership::External;
    let pro = ProPromptRuntime::capture(
        temp.path().to_owned(),
        temp.path().join("absent.sqlite"),
        plugins::installed(temp.path()),
        server.clone(),
        Some(external_config),
        connected.status.clone(),
    )
    .await;
    let err = pro
        .prepare("!pro external status missing", "thread-b")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("connection did not become ready"), "{err}");
    connected.close().await;
    server.close().await.unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn external_ownership_mode_rejects_fresh_process_heartbeat_when_bridge_connection_liveness_is_stale()
 {
    let temp = tempfile::tempdir().unwrap();
    let server = Arc::new(
        crate::test_support::app_fixture::start_fake_server(&temp, &temp.path().join("rpc.jsonl"))
            .await,
    );

    let connected = connection::Connection::start().await;
    let mut external_config = connected.config.clone();
    external_config.ownership = cdr_remote_agent::config::RemoteMcpOwnership::External;

    let current_exe = std::env::current_exe().unwrap();
    let pid = std::process::id();
    let process_identity = cdr_windows_native::current_process_identity().unwrap();
    let exe_sha = cdr_remote_agent::status::sha256_file_hex(&current_exe).unwrap();

    let maint_dir = temp.path().join(".cdr-maintenance");
    std::fs::create_dir_all(&maint_dir).unwrap();
    let now_ms = chrono::Utc::now().timestamp_millis();
    std::fs::write(
        maint_dir.join("mcp-ownership-state.json"),
        serde_json::json!({
            "schema_version": 1,
            "mode": "external",
            "state": "committed",
            "committed": true,
            "owner_pid": pid,
            "owner_identity": process_identity,
            "updated_at_ms": now_ms,
        })
        .to_string(),
    )
    .unwrap();

    let bot_data = temp.path().join("bot_data");
    std::fs::create_dir_all(&bot_data).unwrap();
    let status_path = bot_data.join("remote_mcp_status.json");
    let stale_conn_ms = now_ms - 8_000;
    let stale_record = cdr_remote_agent::status::ExternalMcpStatusRecord {
        schema_version: cdr_remote_agent::status::EXTERNAL_MCP_STATUS_SCHEMA_VERSION,
        component: "mcp".into(),
        install_root: cdr_remote_agent::status::normalize_install_root(temp.path()),
        process_identity: process_identity.clone(),
        pid,
        executable_path: current_exe.display().to_string(),
        artifact_sha256: exe_sha,
        build_info: cdr_remote_agent::status::ExternalMcpBuildInfo {
            schema_version: 1,
            package_version: "0.1.0".into(),
            source_commit: None,
            source_tree: None,
            source_state: "clean".into(),
        },
        // Simulate a live local process rewriting heartbeat_at_ms every second while bridge connection is stale
        updated_at_ms: stale_conn_ms,
        heartbeat_at_ms: now_ms,
        connection_verified_at_ms: Some(stale_conn_ms),
        device_id: external_config.device_id.clone(),
        bridge_fingerprint: external_config.bridge_fingerprint(),
        hello_ack: true,
        ready: true,
        draining: false,
        generation: 1,
    };
    cdr_remote_agent::status::write_external_mcp_status_atomic(&status_path, &stale_record)
        .unwrap();

    let pro = ProPromptRuntime::capture(
        temp.path().to_owned(),
        temp.path().join("state.sqlite"),
        plugins::installed(temp.path()),
        server.clone(),
        Some(external_config),
        connected.status.clone(),
    )
    .await;

    let err = pro
        .prepare(
            "!pro stale bridge connection with fresh heartbeat",
            "thread-b",
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("connection did not become ready"), "{err}");

    // Even if updated_at_ms were spoofed to now_ms while connection_verified_at_ms is stale, !pro must still refuse
    let mut spoofed_updated = stale_record.clone();
    spoofed_updated.updated_at_ms = chrono::Utc::now().timestamp_millis();
    spoofed_updated.heartbeat_at_ms = spoofed_updated.updated_at_ms;
    cdr_remote_agent::status::write_external_mcp_status_atomic(&status_path, &spoofed_updated)
        .unwrap();
    let err2 = pro
        .prepare("!pro stale connection_verified_at_ms", "thread-b")
        .await
        .unwrap_err()
        .to_string();
    assert!(err2.contains("connection did not become ready"), "{err2}");

    connected.close().await;
    server.close().await.unwrap();
}
