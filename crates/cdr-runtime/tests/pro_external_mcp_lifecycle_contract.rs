#![cfg(windows)]
#![allow(clippy::too_many_lines)]

#[allow(dead_code)]
#[path = "support/action_app_server.rs"]
mod app_fixture;
#[path = "support/pro_plugins.rs"]
mod pro_plugins;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use cdr_remote_agent::config::{RemoteMcpConfig, RemoteMcpOwnership, load_remote_mcp_config};
use cdr_remote_agent::status::{
    EXTERNAL_MCP_STATUS_SCHEMA_VERSION, ExternalMcpBuildInfo, ExternalMcpStatusRecord,
    RemoteAgentStatus, write_external_mcp_status_atomic,
};
use cdr_runtime::action_executor::ActionExecutor;
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_runtime::config::{CliOptions, ConfigError, RuntimeConfig};
use cdr_runtime::pro_runtime::{
    ProPromptRuntime, effective_remote_mcp_ownership, verify_external_ownership_state_committed,
};
use cdr_runtime::prompt_preprocessor::PromptPreprocessor;
use cdr_runtime::queue_runner::{BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord};
use cdr_store::mapping::upsert_thread;
use cdr_windows_native::current_process_identity;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

struct OwnedChildGuard {
    child: Child,
}

impl OwnedChildGuard {
    fn new(child: Child) -> Self {
        Self { child }
    }

    fn id(&self) -> u32 {
        self.child.id()
    }

    fn kill_and_wait(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn inner_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}

impl Drop for OwnedChildGuard {
    fn drop(&mut self) {
        self.kill_and_wait();
    }
}

#[derive(Default)]
struct RecordingTurnBackend {
    starts: Mutex<Vec<String>>,
}

impl TurnBackend for RecordingTurnBackend {
    fn generation(&self) -> u64 {
        1
    }

    fn requires_app_server_fork(&self) -> bool {
        false
    }

    fn active_turn_id<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }

    fn read_turns<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn resume_thread<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn fork_thread<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, String> {
        Box::pin(async move { Ok(thread_id.to_string()) })
    }

    fn start_turn<'a>(
        &'a self,
        _thread_id: &'a str,
        prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.starts.lock().await.push(prompt.to_string());
            Ok(format!("turn-{}", self.starts.lock().await.len()))
        })
    }
}

fn ensure_mcp_bin() -> PathBuf {
    static BUILT_MCP_BIN: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT_MCP_BIN
        .get_or_init(|| {
            let runtime_bin = PathBuf::from(env!("CARGO_BIN_EXE_cdr-runtime"));
            let candidate = runtime_bin.with_file_name("cdr-remote-agent.exe");
            let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(|p| p.parent())
                .expect("workspace root");
            let status = Command::new("cargo")
                .args([
                    "build",
                    "--locked",
                    "-p",
                    "cdr-remote-agent",
                    "--bin",
                    "cdr-remote-agent",
                ])
                .current_dir(workspace_root)
                .status()
                .expect("build cdr-remote-agent");
            assert!(status.success(), "cargo build cdr-remote-agent failed");
            assert!(
                candidate.exists(),
                "cdr-remote-agent.exe missing at {}",
                candidate.display()
            );
            let build_info = Command::new(&candidate)
                .arg("--build-info")
                .output()
                .expect("query cdr-remote-agent --build-info");
            assert!(build_info.status.success(), "--build-info failed");
            let sha = cdr_remote_agent::status::sha256_file_hex(&candidate)
                .expect("hash freshly built cdr-remote-agent.exe");
            eprintln!(
                "freshly built cdr-remote-agent.exe: path={} sha256={} build_info={}",
                candidate.display(),
                sha,
                String::from_utf8_lossy(&build_info.stdout).trim()
            );
            candidate
        })
        .clone()
}

fn base_bot_env() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("DISCORD_BOT_TOKEN".into(), "bot-token".into()),
        ("DISCORD_ALLOWED_CHANNEL_IDS".into(), "10".into()),
        ("CODEX_REMOTE_MCP_ENABLED".into(), "1".into()),
        (
            "CODEX_REMOTE_MCP_BRIDGE_URL".into(),
            "wss://gateway.example.test/bridge".into(),
        ),
        ("CODEX_REMOTE_MCP_DEVICE_ID".into(), "device-alpha".into()),
        (
            "CODEX_REMOTE_MCP_DEVICE_TOKEN".into(),
            "secret-device-token".into(),
        ),
    ])
}

fn current_exe_record(
    install_root: &Path,
    config: &RemoteMcpConfig,
    generation: u64,
) -> ExternalMcpStatusRecord {
    let exe = std::env::current_exe().expect("current_exe");
    let now_ms = chrono::Utc::now().timestamp_millis();
    ExternalMcpStatusRecord {
        schema_version: EXTERNAL_MCP_STATUS_SCHEMA_VERSION,
        component: "mcp".into(),
        install_root: cdr_remote_agent::status::normalize_install_root(install_root),
        process_identity: current_process_identity().expect("process identity"),
        pid: std::process::id(),
        executable_path: exe.display().to_string(),
        artifact_sha256: cdr_remote_agent::status::sha256_file_hex(&exe).expect("hash exe"),
        build_info: ExternalMcpBuildInfo {
            schema_version: 1,
            package_version: "0.1.0".into(),
            source_commit: None,
            source_tree: None,
            source_state: "unknown".into(),
        },
        updated_at_ms: now_ms,
        heartbeat_at_ms: now_ms,
        connection_verified_at_ms: Some(now_ms),
        device_id: config.device_id.clone(),
        bridge_fingerprint: config.bridge_fingerprint(),
        hello_ack: true,
        ready: true,
        draining: false,
        generation,
    }
}

fn setup_state_db(root: &Path) -> PathBuf {
    let project = root.join("project-workspace");
    fs::create_dir_all(&project).expect("create project");
    let state_db = root.join("state.sqlite");
    let conn = rusqlite::Connection::open(&state_db).expect("open state db");
    conn.execute_batch(include_str!("fixtures/action_state.sql"))
        .expect("apply action_state.sql");
    for thread_id in [
        "thread-b",
        "thread-b1",
        "thread-b2",
        "thread-b3",
        "thread-b4",
        "thread-b5",
        "thread-b6",
    ] {
        conn.execute(
            "INSERT OR REPLACE INTO threads (id, title, cwd, updated_at, rollout_path, tokens_used, archived) VALUES (?1, 'Test Thread', ?2, 1, '', 0, 0)",
            [thread_id, project.to_str().unwrap()],
        )
        .expect("insert thread");
    }
    state_db
}

async fn wait_for_status(
    path: &Path,
    predicate: impl Fn(&ExternalMcpStatusRecord) -> bool,
) -> ExternalMcpStatusRecord {
    tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            if let Ok(bytes) = fs::read(path)
                && let Ok(record) = serde_json::from_slice::<ExternalMcpStatusRecord>(&bytes)
                && predicate(&record)
            {
                return record;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("timed out waiting for external MCP status predicate")
}

#[tokio::test]
async fn stage_a_ownership_mode_parses_explicit_embedded_and_external_and_rejects_invalid_or_uncommitted_state()
 {
    let env = base_bot_env();
    let default_cfg = RuntimeConfig::from_map(&env, CliOptions::default()).expect("default config");
    assert_eq!(
        default_cfg.remote_mcp.as_ref().unwrap().ownership,
        RemoteMcpOwnership::Embedded
    );

    let mut external_env = base_bot_env();
    external_env.insert("CODEX_REMOTE_MCP_OWNERSHIP".into(), "external".into());
    external_env.insert("CODEX_MCP_DATA_DIR".into(), "custom_mcp_dir".into());
    external_env.insert(
        "CODEX_REMOTE_MCP_STATUS_FILE".into(),
        "custom_mcp_dir/my_status.json".into(),
    );
    let external_cfg =
        RuntimeConfig::from_map(&external_env, CliOptions::default()).expect("external config");
    let mcp_cfg = external_cfg.remote_mcp.as_ref().unwrap();
    assert_eq!(mcp_cfg.ownership, RemoteMcpOwnership::External);

    let temp = tempfile::tempdir().expect("tempdir");
    assert_eq!(
        mcp_cfg.resolve_data_dir(temp.path()),
        temp.path().join("custom_mcp_dir")
    );
    assert_eq!(
        mcp_cfg.resolve_status_file_path(temp.path()),
        temp.path().join("custom_mcp_dir/my_status.json")
    );

    let mut invalid_env = base_bot_env();
    invalid_env.insert("CODEX_REMOTE_MCP_OWNERSHIP".into(), "hybrid".into());
    let err = RuntimeConfig::from_map(&invalid_env, CliOptions::default()).unwrap_err();
    assert!(matches!(err, ConfigError::RemoteMcp(_)));

    // Absence of mcp-ownership-state.json preserves configured Embedded mode
    assert_eq!(
        effective_remote_mcp_ownership(temp.path(), RemoteMcpOwnership::Embedded),
        RemoteMcpOwnership::Embedded
    );
    assert!(
        verify_external_ownership_state_committed(temp.path(), RemoteMcpOwnership::External)
            .is_ok()
    );
    assert!(
        verify_external_ownership_state_committed(temp.path(), RemoteMcpOwnership::Embedded)
            .is_err()
    );

    let maint_dir = temp.path().join(".cdr-maintenance");
    fs::create_dir_all(&maint_dir).unwrap();
    let ownership_file = maint_dir.join("mcp-ownership-state.json");

    // Malformed / corrupt ownership state must NEVER revive Embedded MCP alongside an external owner,
    // and must fail-close verify_external_ownership_state_committed
    fs::write(&ownership_file, "{corrupt json").unwrap();
    assert_eq!(
        effective_remote_mcp_ownership(temp.path(), RemoteMcpOwnership::Embedded),
        RemoteMcpOwnership::External
    );
    assert!(
        verify_external_ownership_state_committed(temp.path(), RemoteMcpOwnership::Embedded)
            .is_err()
    );

    // Uncommitted cutover_pending or cutover_failed_hold state must also fence Embedded revival and fail-close !pro
    fs::write(
        &ownership_file,
        r#"{"schema_version":1,"mode":"external","state":"cutover_failed_hold","committed":false}"#,
    )
    .unwrap();
    assert_eq!(
        effective_remote_mcp_ownership(temp.path(), RemoteMcpOwnership::Embedded),
        RemoteMcpOwnership::External
    );
    assert!(
        verify_external_ownership_state_committed(temp.path(), RemoteMcpOwnership::Embedded)
            .is_err()
    );

    // Missing schema_version or contradictory state/committed fields must also fail closed
    for invalid_state in [
        r#"{"mode":"external","state":"committed","committed":true}"#,
        r#"{"schema_version":2,"mode":"external","state":"committed","committed":true}"#,
        r#"{"schema_version":1,"mode":"external","state":"cutover_pending","committed":true}"#,
        r#"{"schema_version":1,"mode":"external","state":"committed","committed":false}"#,
    ] {
        fs::write(&ownership_file, invalid_state).unwrap();
        assert_eq!(
            effective_remote_mcp_ownership(temp.path(), RemoteMcpOwnership::Embedded),
            RemoteMcpOwnership::External
        );
        assert!(
            verify_external_ownership_state_committed(temp.path(), RemoteMcpOwnership::Embedded)
                .is_err()
        );
    }

    // Committed external state enables external ownership and passes verification
    fs::write(
        &ownership_file,
        r#"{"schema_version":1,"mode":"external","state":"committed","committed":true}"#,
    )
    .unwrap();
    assert_eq!(
        effective_remote_mcp_ownership(temp.path(), RemoteMcpOwnership::Embedded),
        RemoteMcpOwnership::External
    );
    assert!(
        verify_external_ownership_state_committed(temp.path(), RemoteMcpOwnership::Embedded)
            .is_ok()
    );

    // Verify ProPromptRuntime::capture honors custom CODEX_REMOTE_MCP_STATUS_FILE
    let custom_status_path = mcp_cfg.resolve_status_file_path(temp.path());
    fs::create_dir_all(custom_status_path.parent().unwrap()).unwrap();
    let state_db = setup_state_db(temp.path());
    let codex_exe = pro_plugins::installed(temp.path());
    let server =
        Arc::new(app_fixture::start_fake_server(&temp, &temp.path().join("rpc.jsonl")).await);
    let runtime = ProPromptRuntime::capture(
        temp.path().to_path_buf(),
        state_db,
        codex_exe,
        Arc::clone(&server),
        Some(mcp_cfg.clone()),
        RemoteAgentStatus::default(),
    )
    .await;
    let rec = current_exe_record(temp.path(), mcp_cfg, 1);
    write_external_mcp_status_atomic(&custom_status_path, &rec).unwrap();
    let prepared = runtime
        .prepare("!pro custom-path-check", "thread-b")
        .await
        .expect("custom status path resolved by ProPromptRuntime::capture");
    assert!(prepared.contains("device_id=\"device-alpha\""));
    server.close().await.unwrap();
}

#[tokio::test]
async fn t05_external_mcp_down_and_restart_recovers_pro_without_bot_restart_and_detects_epoch_change()
 {
    let mcp_exe = ensure_mcp_bin();
    let temp = tempfile::tempdir().expect("tempdir");
    let data_dir = temp.path().join("bot_data");
    fs::create_dir_all(&data_dir).unwrap();
    let status_path = data_dir.join("remote_mcp_status.json");
    let stop_path = data_dir.join("remote_mcp.stop");
    let local_mcp_exe = temp.path().join("cdr-remote-agent.exe");
    fs::copy(&mcp_exe, &local_mcp_exe).expect("copy cdr-remote-agent.exe to temp");
    let mcp_exe = local_mcp_exe;
    let state_db = setup_state_db(temp.path());
    let mirror_db = temp.path().join("mirror.sqlite");
    let codex_exe = pro_plugins::installed(temp.path());
    let server =
        Arc::new(app_fixture::start_fake_server(&temp, &temp.path().join("rpc.jsonl")).await);

    // Start local WebSocket mock bridge server that accepts consecutive real cdr-remote-agent.exe connections
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind bridge");
    let bridge_addr = listener.local_addr().expect("bridge addr");
    let bridge_task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                if let Ok(mut ws) = accept_async(stream).await
                    && let Some(Ok(hello)) = ws.next().await
                    && let Ok(text) = hello.to_text()
                    && text.contains("\"type\":\"hello\"")
                {
                    let _ = ws
                        .send(Message::Text(
                            r#"{"type":"hello_ack","protocol_version":10}"#.into(),
                        ))
                        .await;
                    while let Some(Ok(msg)) = ws.next().await {
                        if let Message::Ping(payload) = msg {
                            let _ = ws.send(Message::Pong(payload)).await;
                        }
                    }
                }
            });
        }
    });

    let env_file = temp.path().join(".env");
    fs::write(
        &env_file,
        format!(
            "CODEX_REMOTE_MCP_ENABLED=1\n\
             CODEX_REMOTE_MCP_OWNERSHIP=external\n\
             CODEX_REMOTE_MCP_BRIDGE_URL=ws://{bridge_addr}\n\
             CODEX_REMOTE_MCP_DEVICE_ID=device-alpha\n\
             CODEX_REMOTE_MCP_DEVICE_TOKEN=secret-device-token\n"
        ),
    )
    .expect("write .env");

    let mut env = base_bot_env();
    env.insert("CODEX_REMOTE_MCP_OWNERSHIP".into(), "external".into());
    env.insert(
        "CODEX_REMOTE_MCP_BRIDGE_URL".into(),
        format!("ws://{bridge_addr}"),
    );
    let remote_cfg = load_remote_mcp_config(&env).unwrap().unwrap();
    let bot_identity_before = current_process_identity().expect("bot process identity");

    let pro_runtime = Arc::new(
        ProPromptRuntime::capture(
            temp.path().to_path_buf(),
            state_db.clone(),
            codex_exe,
            Arc::clone(&server),
            Some(remote_cfg.clone()),
            RemoteAgentStatus::default(),
        )
        .await,
    );
    let turn_backend = Arc::new(RecordingTurnBackend::default());
    let queue = Arc::new(QueueCoordinator::new(
        mirror_db.clone(),
        Arc::clone(&turn_backend),
    ));
    let executor = ActionExecutor::new(
        state_db,
        mirror_db,
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        queue,
    )
    .with_prompt_preprocessor(Arc::clone(&pro_runtime) as Arc<dyn PromptPreprocessor>);
    for (channel_id, thread_id) in [
        (10, "thread-b1"),
        (11, "thread-b2"),
        (12, "thread-b3"),
        (13, "thread-b4"),
        (14, "thread-b5"),
        (15, "thread-b6"),
    ] {
        upsert_thread(
            executor.mirror_db(),
            thread_id,
            "project-workspace",
            "Beta",
            9,
            channel_id,
            1.0,
        )
        .unwrap();
    }

    // 1. Before external MCP starts: representative non-!pro Discord ask succeeds, while !pro fails closed
    executor
        .execute(
            CommandAction::Ask {
                prompt: "일반 질문 1".into(),
            },
            10,
            20,
        )
        .await
        .expect("regular Discord ask succeeds while MCP is offline");
    assert_eq!(turn_backend.starts.lock().await.len(), 1);

    let err = executor
        .execute(
            CommandAction::Ask {
                prompt: "!pro 확인".into(),
            },
            11,
            20,
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("connection did not become ready"));
    assert_eq!(turn_backend.starts.lock().await.len(), 1);

    // 2. Spawn REAL cdr-remote-agent.exe #1 -> connects to mock bridge -> writes ready status -> !pro succeeds
    let mut mcp_child_1 = OwnedChildGuard::new(
        Command::new(&mcp_exe)
            .arg("--env")
            .arg(&env_file)
            .current_dir(temp.path())
            .env("CODEX_REMOTE_MCP_RECONNECT_SECONDS", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn cdr-remote-agent #1"),
    );
    let first_status = wait_for_status(&status_path, |s| {
        s.ready && s.hello_ack && s.pid == mcp_child_1.id()
    })
    .await;

    executor
        .execute(
            CommandAction::Ask {
                prompt: "!pro 첫 연결 확인".into(),
            },
            11,
            20,
        )
        .await
        .expect("!pro succeeds when real cdr-remote-agent #1 is ready");
    assert_eq!(turn_backend.starts.lock().await.len(), 2);
    assert!(turn_backend.starts.lock().await[1].contains("device_id=\"device-alpha\""));
    let epoch_before = pro_runtime
        .current_connection_epoch()
        .await
        .expect("epoch 1");
    assert_eq!(epoch_before.0, first_status.process_identity);

    // 3. Gracefully stop real cdr-remote-agent.exe #1 via stop file -> bot runtime stays alive & processes regular Discord turns, while !pro fails closed
    fs::write(&stop_path, "stop\n").expect("write stop file");
    let exit_status_1 = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(st) = mcp_child_1.inner_mut().try_wait().expect("try_wait") {
                break st;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("mcp_child_1 exited cleanly");
    assert!(exit_status_1.success());
    let _ = fs::remove_file(&stop_path);

    // Regular Discord command still works with identical bot identity
    executor
        .execute(
            CommandAction::Ask {
                prompt: "MCP 중단 중 일반 질문".into(),
            },
            12,
            20,
        )
        .await
        .expect("regular Discord ask succeeds after external MCP stop");
    assert_eq!(turn_backend.starts.lock().await.len(), 3);
    assert_eq!(
        current_process_identity().expect("bot identity"),
        bot_identity_before
    );

    // !pro fails closed immediately while external MCP #1 is stopped
    let err_stopped = executor
        .execute(
            CommandAction::Ask {
                prompt: "!pro 중단중".into(),
            },
            13,
            20,
        )
        .await
        .unwrap_err();
    assert!(
        err_stopped
            .to_string()
            .contains("connection did not become ready")
    );

    // 4. Spawn REAL cdr-remote-agent.exe #2 (new process_identity, generation=1) -> !pro recovers automatically without bot restart
    let mut mcp_child_2 = OwnedChildGuard::new(
        Command::new(&mcp_exe)
            .arg("--env")
            .arg(&env_file)
            .current_dir(temp.path())
            .env("CODEX_REMOTE_MCP_RECONNECT_SECONDS", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn cdr-remote-agent #2"),
    );
    let second_status = wait_for_status(&status_path, |s| {
        s.ready && s.hello_ack && s.pid == mcp_child_2.id()
    })
    .await;
    assert_ne!(
        second_status.process_identity,
        first_status.process_identity
    );

    executor
        .execute(
            CommandAction::Ask {
                prompt: "!pro 복구 확인".into(),
            },
            13,
            20,
        )
        .await
        .expect("!pro recovers automatically when real cdr-remote-agent #2 connects");
    assert_eq!(turn_backend.starts.lock().await.len(), 4);
    assert!(turn_backend.starts.lock().await[3].contains("device_id=\"device-alpha\""));

    // 5. Mid-turn epoch check detects that process_identity changed even though generation is 1
    assert!(
        pro_runtime
            .verify_epoch_unchanged(&epoch_before)
            .await
            .is_err()
    );

    // 6. Crash/kill real cdr-remote-agent.exe #2 and verify !pro fails closed even before status file is updated (via live process identity + lock check)
    mcp_child_2.kill_and_wait();
    let err_crashed = executor
        .execute(
            CommandAction::Ask {
                prompt: "!pro 크래시 직후".into(),
            },
            14,
            20,
        )
        .await
        .unwrap_err();
    assert!(
        err_crashed
            .to_string()
            .contains("connection did not become ready")
    );

    // 7. Replaying retired process #1's status record is rejected
    write_external_mcp_status_atomic(&status_path, &first_status).expect("replay retired status");
    let replay_err = pro_runtime
        .prepare("!pro 확인", "thread-b")
        .await
        .unwrap_err();
    assert!(
        replay_err
            .to_string()
            .contains("connection did not become ready")
    );

    bridge_task.abort();
    server.close().await.unwrap();
}

#[tokio::test]
async fn t10_pro_external_mcp_status_rejects_stale_mismatched_draining_unacked_and_wrong_identity()
{
    let temp = tempfile::tempdir().expect("tempdir");
    let status_path = temp.path().join("bot_data").join("remote_mcp_status.json");
    fs::create_dir_all(status_path.parent().unwrap()).unwrap();
    let state_db = setup_state_db(temp.path());
    let codex_exe = pro_plugins::installed(temp.path());
    let server =
        Arc::new(app_fixture::start_fake_server(&temp, &temp.path().join("rpc.jsonl")).await);

    let mut env = base_bot_env();
    env.insert("CODEX_REMOTE_MCP_OWNERSHIP".into(), "external".into());
    let remote_cfg = load_remote_mcp_config(&env).unwrap().unwrap();
    let runtime = ProPromptRuntime::capture(
        temp.path().to_path_buf(),
        state_db,
        codex_exe,
        Arc::clone(&server),
        Some(remote_cfg.clone()),
        RemoteAgentStatus::default(),
    )
    .await;

    let valid = current_exe_record(temp.path(), &remote_cfg, 3);

    // 1. Stale heartbeat/updated_at
    let mut stale = valid.clone();
    stale.updated_at_ms -= 30_000;
    stale.heartbeat_at_ms -= 30_000;
    stale.connection_verified_at_ms = Some(stale.updated_at_ms);
    write_external_mcp_status_atomic(&status_path, &stale).unwrap();
    assert!(runtime.prepare("!pro stale", "thread-b").await.is_err());

    // 1b. R3: Fresh process heartbeat_at_ms, but stale/future/missing/null bridge connection_verified_at_ms
    let mut stale_conn = valid.clone();
    stale_conn.updated_at_ms -= 30_000;
    stale_conn.connection_verified_at_ms = Some(stale_conn.updated_at_ms);
    write_external_mcp_status_atomic(&status_path, &stale_conn).unwrap();
    assert!(
        runtime
            .prepare("!pro stale-bridge-conn", "thread-b")
            .await
            .is_err()
    );
    let mut future_conn = valid.clone();
    future_conn.connection_verified_at_ms = Some(valid.heartbeat_at_ms + 30_000);
    write_external_mcp_status_atomic(&status_path, &future_conn).unwrap();
    assert!(
        runtime
            .prepare("!pro future-bridge-conn", "thread-b")
            .await
            .is_err()
    );
    let mut missing_conn = valid.clone();
    missing_conn.connection_verified_at_ms = None;
    write_external_mcp_status_atomic(&status_path, &missing_conn).unwrap();
    assert!(
        runtime
            .prepare("!pro missing-bridge-conn", "thread-b")
            .await
            .is_err()
    );
    let mut null_conn_json: serde_json::Value = serde_json::to_value(&valid).unwrap();
    null_conn_json["connection_verified_at_ms"] = serde_json::Value::Null;
    fs::write(&status_path, serde_json::to_vec(&null_conn_json).unwrap()).unwrap();
    assert!(
        runtime
            .prepare("!pro null-bridge-conn", "thread-b")
            .await
            .is_err()
    );

    // 2. Mismatched device_id
    let mut wrong_device = valid.clone();
    wrong_device.device_id = "device-beta".into();
    write_external_mcp_status_atomic(&status_path, &wrong_device).unwrap();
    assert!(
        runtime
            .prepare("!pro wrong-device", "thread-b")
            .await
            .is_err()
    );

    // 3. Mismatched bridge_fingerprint
    let mut wrong_bridge = valid.clone();
    wrong_bridge.bridge_fingerprint = "0000000000000000".into();
    write_external_mcp_status_atomic(&status_path, &wrong_bridge).unwrap();
    assert!(
        runtime
            .prepare("!pro wrong-bridge", "thread-b")
            .await
            .is_err()
    );

    // 4. Missing hello_ack
    let mut no_ack = valid.clone();
    no_ack.hello_ack = false;
    write_external_mcp_status_atomic(&status_path, &no_ack).unwrap();
    assert!(runtime.prepare("!pro no-ack", "thread-b").await.is_err());

    // 5. Draining state even if ready=true in tampered file
    let mut draining = valid.clone();
    draining.draining = true;
    write_external_mcp_status_atomic(&status_path, &draining).unwrap();
    assert!(runtime.prepare("!pro draining", "thread-b").await.is_err());

    // 6. Mismatched process_identity (PID reuse / wrong start ticks)
    let mut wrong_identity = valid.clone();
    wrong_identity.process_identity = format!("{}|1234567890", std::process::id());
    write_external_mcp_status_atomic(&status_path, &wrong_identity).unwrap();
    assert!(
        runtime
            .prepare("!pro wrong-identity", "thread-b")
            .await
            .is_err()
    );

    // 7. Mismatched active instance lock owner
    write_external_mcp_status_atomic(&status_path, &valid).unwrap();
    fs::write(
        temp.path()
            .join("bot_data")
            .join("remote_mcp.instance.lock"),
        "999999|9999999990\n",
    )
    .unwrap();
    assert!(
        runtime
            .prepare("!pro lock-mismatch", "thread-b")
            .await
            .is_err()
    );

    server.close().await.unwrap();
}
