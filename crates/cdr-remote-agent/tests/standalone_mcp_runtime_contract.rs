#![allow(clippy::too_many_lines)]

use std::fs;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

const MAX_PIPE_CAPTURE_BYTES: usize = 64 * 1024;

fn spawn_pipe_drainer<R: Read + Send + 'static>(
    mut reader: R,
    sink: Arc<Mutex<Vec<u8>>>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut chunk = [0_u8; 4096];
        while let Ok(read) = reader.read(&mut chunk) {
            if read == 0 {
                break;
            }
            if let Ok(mut buf) = sink.lock() {
                let remaining = MAX_PIPE_CAPTURE_BYTES.saturating_sub(buf.len());
                if remaining > 0 {
                    let take = remaining.min(read);
                    buf.extend_from_slice(&chunk[..take]);
                }
            }
        }
    })
}

struct OwnedChildGuard {
    child: Child,
    stderr_capture: Arc<Mutex<Vec<u8>>>,
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
}

impl OwnedChildGuard {
    fn new(mut child: Child) -> Self {
        let stdout_capture = Arc::new(Mutex::new(Vec::new()));
        let stderr_capture = Arc::new(Mutex::new(Vec::new()));
        let stdout_thread = child
            .stdout
            .take()
            .map(|out| spawn_pipe_drainer(out, stdout_capture));
        let stderr_thread = child
            .stderr
            .take()
            .map(|err| spawn_pipe_drainer(err, Arc::clone(&stderr_capture)));
        Self {
            child,
            stderr_capture,
            stdout_thread,
            stderr_thread,
        }
    }

    fn id(&self) -> u32 {
        self.child.id()
    }

    fn inner_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    fn stderr_string(&mut self) -> String {
        if let Some(handle) = self.stdout_thread.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.stderr_thread.take() {
            let _ = handle.join();
        }
        let guard = self.stderr_capture.lock().expect("stderr lock");
        String::from_utf8_lossy(&guard).into_owned()
    }
}

impl Drop for OwnedChildGuard {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if let Some(handle) = self.stdout_thread.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.stderr_thread.take() {
            let _ = handle.join();
        }
    }
}

fn clean_mcp_command() -> Command {
    let mut cmd = Command::new(mcp_bin());
    for (key, _) in std::env::vars_os() {
        if let Some(name) = key.to_str()
            && (name.starts_with("CODEX_REMOTE_MCP_")
                || name.starts_with("DISCORD_")
                || name == "CODEX_MCP_DATA_DIR"
                || name == "CODEX_BOT_DATA_DIR")
        {
            cmd.env_remove(name);
        }
    }
    cmd
}

use cdr_remote_agent::config::{RemoteMcpOwnership, load_remote_mcp_config};
use cdr_remote_agent::dispatcher::LocalProjectDispatcher;
use cdr_remote_agent::restart_handoff::RestartHandoffRuntime;
use cdr_remote_agent::status::{EXTERNAL_MCP_STATUS_SCHEMA_VERSION, ExternalMcpStatusRecord};
use cdr_remote_protocol::message::{
    GatewayCommand, GatewayInboundMessage, ProtocolV10, parse_bridge_message,
};
use cdr_remote_protocol::request::{ProjectOperation, TerminalRequest, TerminalShell};
use chrono::{Duration as ChronoDuration, Utc};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

fn mcp_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cdr-remote-agent")
}

#[test]
fn standalone_mcp_cli_supports_help_build_info_and_check_config_without_discord_token() {
    let help = clean_mcp_command()
        .arg("--help")
        .output()
        .expect("run --help");
    assert!(help.status.success());
    let help_text = String::from_utf8_lossy(&help.stdout);
    assert!(help_text.contains("cdr-remote-agent"));
    assert!(help_text.contains("--check-config"));

    let build_info = clean_mcp_command()
        .arg("--build-info")
        .output()
        .expect("run --build-info");
    assert!(build_info.status.success());
    let build_json: serde_json::Value =
        serde_json::from_slice(&build_info.stdout).expect("valid build-info json");
    assert_eq!(build_json["schema_version"], 1);

    let temp = tempfile::tempdir().expect("tempdir");
    let env_file = temp.path().join(".env");
    fs::write(
        &env_file,
        "CODEX_REMOTE_MCP_ENABLED=1\n\
         CODEX_REMOTE_MCP_BRIDGE_URL=ws://127.0.0.1:8030/bridge\n\
         CODEX_REMOTE_MCP_DEVICE_ID=device-standalone\n\
         CODEX_REMOTE_MCP_DEVICE_TOKEN=secret-token-value\n",
    )
    .expect("write env");

    let check = clean_mcp_command()
        .arg("--check-config")
        .arg("--env")
        .arg(&env_file)
        .output()
        .expect("run --check-config");
    assert!(
        check.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        !temp
            .path()
            .join("bot_data")
            .join("bot_state.sqlite3")
            .exists()
    );

    // Inherited empty environment variable must preserve existing environment key precedence
    // (matching cdr-runtime::config::merge_env_text) instead of silently filling from .env.
    let empty_override = clean_mcp_command()
        .arg("--check-config")
        .arg("--env")
        .arg(&env_file)
        .env("CODEX_REMOTE_MCP_DEVICE_ID", "   ")
        .output()
        .expect("run --check-config with blank inherited key");
    assert!(
        !empty_override.status.success(),
        "Blank inherited CODEX_REMOTE_MCP_DEVICE_ID must not be silently replaced by .env"
    );
}

#[tokio::test]
async fn standalone_mcp_writes_atomic_status_and_handles_disconnect_and_handoff_drain() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).expect("create project");
    let data_dir = temp.path().join("bot_data");
    fs::create_dir_all(&data_dir).expect("create bot_data");
    let status_file = data_dir.join("remote_mcp_status.json");
    let stop_file = data_dir.join("remote_mcp.stop");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("addr");
    let env_file = temp.path().join(".env");
    let env_content = format!(
        "CODEX_REMOTE_MCP_ENABLED=1\n\
         CODEX_REMOTE_MCP_OWNERSHIP=external\n\
         CODEX_REMOTE_MCP_BRIDGE_URL=ws://{address}\n\
         CODEX_REMOTE_MCP_DEVICE_ID=device-standalone\n\
         CODEX_REMOTE_MCP_DEVICE_TOKEN=secret-token-value\n"
    );
    fs::write(&env_file, &env_content).expect("write env");

    let (drop_first_tx, drop_first_rx) = tokio::sync::oneshot::channel::<()>();
    let (reconnected_tx, reconnected_rx) = tokio::sync::oneshot::channel::<()>();
    let project_for_server = project_dir.clone();
    let server = tokio::spawn(async move {
        // First connection: Hello -> HelloAck -> wait for drop signal -> close
        let (stream, _) = listener.accept().await.expect("accept 1");
        let mut socket = accept_async(stream).await.expect("ws 1");
        let hello = socket.next().await.expect("hello 1").expect("msg 1");
        parse_bridge_message(hello.to_text().expect("text")).expect("parse hello");
        socket
            .send(Message::Text(
                serde_json::to_string(&GatewayInboundMessage::HelloAck {
                    protocol_version: ProtocolV10,
                })
                .unwrap()
                .into(),
            ))
            .await
            .expect("send ack 1");
        let _ = drop_first_rx.await;
        let _ = socket.close(None).await;
        drop(socket);

        // Second connection after delay: Hello -> HelloAck -> bind project session -> wait until client closes
        tokio::time::sleep(Duration::from_millis(450)).await;
        let (stream2, _) = listener.accept().await.expect("accept 2");
        let mut socket2 = accept_async(stream2).await.expect("ws 2");
        let hello2 = socket2.next().await.expect("hello 2").expect("msg 2");
        parse_bridge_message(hello2.to_text().expect("text")).expect("parse hello 2");
        socket2
            .send(Message::Text(
                serde_json::to_string(&GatewayInboundMessage::HelloAck {
                    protocol_version: ProtocolV10,
                })
                .unwrap()
                .into(),
            ))
            .await
            .expect("send ack 2");
        socket2
            .send(Message::Text(
                serde_json::to_string(&GatewayInboundMessage::Command(
                    GatewayCommand::DeviceSession {
                        request_id: "req-1".into(),
                        thread_id: "thread-standalone".into(),
                        deadline_at: Utc::now() + ChronoDuration::minutes(1),
                        computer_session_id: "session-standalone".into(),
                        computer_session_generation: 1,
                        working_directory: project_for_server.display().to_string(),
                        expires_at: Utc::now() + ChronoDuration::minutes(10),
                    },
                ))
                .unwrap()
                .into(),
            ))
            .await
            .expect("send session");
        let _ = socket2.next().await.expect("session result").expect("ok");
        let _ = reconnected_tx.send(());
        while let Some(Ok(_)) = socket2.next().await {}
    });

    let mut child = OwnedChildGuard::new(
        clean_mcp_command()
            .arg("--env")
            .arg(&env_file)
            .current_dir(temp.path())
            .env("CODEX_REMOTE_MCP_RECONNECT_SECONDS", "0")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn standalone mcp"),
    );

    // Wait for generation 1 ready status
    let first_status = wait_for_status(&status_file, |s| {
        s.ready && s.hello_ack && s.generation == 1
    })
    .await;
    assert_eq!(
        first_status.schema_version,
        EXTERNAL_MCP_STATUS_SCHEMA_VERSION
    );
    assert_eq!(first_status.component, "mcp");
    assert_eq!(first_status.device_id, "device-standalone");
    assert_eq!(first_status.pid, child.id());
    assert!(!first_status.draining);
    assert!(
        !temp
            .path()
            .join("bot_data")
            .join("bot_state.sqlite3")
            .exists()
    );

    // Duplicate standalone MCP instance must fail closed immediately
    let duplicate = clean_mcp_command()
        .arg("--env")
        .arg(&env_file)
        .current_dir(temp.path())
        .output()
        .expect("spawn duplicate");
    assert!(!duplicate.status.success());

    // Drop first connection and verify status immediately invalidates ready & hello_ack
    let _ = drop_first_tx.send(());
    let disconnected_status = wait_for_status(&status_file, |s| !s.ready && !s.hello_ack).await;
    assert_eq!(
        disconnected_status.process_identity,
        first_status.process_identity
    );

    // Wait for reconnect + session bind (generation 2)
    tokio::time::timeout(Duration::from_secs(5), reconnected_rx)
        .await
        .expect("reconnected")
        .expect("signal");
    let second_status = wait_for_status(&status_file, |s| {
        s.ready && s.hello_ack && s.generation == 2
    })
    .await;
    assert_eq!(
        second_status.process_identity,
        first_status.process_identity
    );

    // Request restart_handoff stop and verify clean exit + handoff persistence
    fs::write(&stop_file, "restart_handoff\n").expect("write stop file");
    let exit_status = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = child.inner_mut().try_wait().expect("try_wait") {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("child exited");
    assert!(exit_status.success());
    server.await.expect("server task");

    let final_record: ExternalMcpStatusRecord =
        serde_json::from_slice(&fs::read(&status_file).expect("read final status"))
            .expect("parse final status");
    assert!(!final_record.ready);
    assert!(!final_record.hello_ack);

    let env_map = std::collections::HashMap::from([
        ("CODEX_REMOTE_MCP_ENABLED".into(), "1".into()),
        ("CODEX_REMOTE_MCP_OWNERSHIP".into(), "external".into()),
        (
            "CODEX_REMOTE_MCP_BRIDGE_URL".into(),
            format!("ws://{address}"),
        ),
        (
            "CODEX_REMOTE_MCP_DEVICE_ID".into(),
            "device-standalone".into(),
        ),
        (
            "CODEX_REMOTE_MCP_DEVICE_TOKEN".into(),
            "secret-token-value".into(),
        ),
    ]);
    let mcp_config = load_remote_mcp_config(&env_map).unwrap().unwrap();
    assert_eq!(mcp_config.ownership, RemoteMcpOwnership::External);
    let handoff = RestartHandoffRuntime::system(temp.path()).expect("system handoff");
    let _ = handoff;
    let stderr_text = child.stderr_string();
    assert!(
        stderr_text.contains("remote_mcp_restart_handoff_prepared projects=1"),
        "stderr: {stderr_text}"
    );
    let dispatcher = LocalProjectDispatcher::new();
    let _ = dispatcher;
}

#[tokio::test]
async fn standalone_mcp_honors_preexisting_stop_intent_and_invalidates_status_before_slow_worker_cleanup()
 {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).expect("create project");
    let data_dir = temp.path().join("custom_mcp_data");
    fs::create_dir_all(&data_dir).expect("create custom_mcp_data");
    let stop_file = data_dir.join("remote_mcp.stop");
    let status_file = data_dir.join("custom_status.json");
    let env_file = temp.path().join(".env");
    fs::write(
        &env_file,
        "CODEX_REMOTE_MCP_ENABLED=1\n\
         CODEX_REMOTE_MCP_OWNERSHIP=external\n\
         CODEX_REMOTE_MCP_BRIDGE_URL=ws://127.0.0.1:9/bridge\n\
         CODEX_REMOTE_MCP_DEVICE_ID=device-stop-check\n\
         CODEX_REMOTE_MCP_DEVICE_TOKEN=secret-token-value\n\
         CODEX_MCP_DATA_DIR=custom_mcp_data\n\
         CODEX_REMOTE_MCP_STATUS_FILE=custom_mcp_data/custom_status.json\n",
    )
    .expect("write env");

    // 1. Pre-existing stop marker must not be deleted on startup; process exits cleanly without connecting
    fs::write(&stop_file, "stop\n").expect("write stop");
    let stopped_out = clean_mcp_command()
        .arg("--env")
        .arg(&env_file)
        .current_dir(temp.path())
        .output()
        .expect("run with stop file");
    assert!(stopped_out.status.success());
    assert!(
        stop_file.exists(),
        "pre-existing stop file must not be deleted on startup"
    );
    fs::remove_file(&stop_file).expect("remove stop file");

    // 2. Pre-existing mcp-intent.json with DesiredState=stopped also prevents startup
    let maint_dir = temp.path().join(".cdr-maintenance");
    fs::create_dir_all(&maint_dir).expect("create maint");
    let intent_file = maint_dir.join("mcp-intent.json");
    fs::write(
        &intent_file,
        r#"{"SchemaVersion":1,"Component":"mcp","Revision":3,"DesiredState":"stopped"}"#,
    )
    .expect("write intent");
    let intent_stopped_out = clean_mcp_command()
        .arg("--env")
        .arg(&env_file)
        .current_dir(temp.path())
        .output()
        .expect("run with stopped intent");
    assert!(intent_stopped_out.status.success());
    assert!(!status_file.exists());
    fs::remove_file(&intent_file).expect("remove intent");

    // 3. Verify serve_socket_until_with_status invalidates status immediately on transport close
    // before an in-flight command worker completes cleanup.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let publisher = cdr_remote_agent::status::ExternalMcpStatusPublisher::new(
        status_file.clone(),
        temp.path(),
        "1234|567890".into(),
        1234,
        std::path::Path::new("cdr-remote-agent.exe"),
        "0".repeat(64),
        cdr_remote_agent::status::ExternalMcpBuildInfo {
            schema_version: 1,
            package_version: "0.1.0".into(),
            source_commit: None,
            source_tree: None,
            source_state: "clean".into(),
        },
        "device-stop-check".into(),
        "fp".into(),
    );
    let status = cdr_remote_agent::status::RemoteAgentStatus::with_external_publisher(publisher);
    let status_for_client = status.clone();
    let project_for_server = project_dir.clone();
    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let mut ws = accept_async(stream).await.expect("ws");
        let _hello = ws.next().await.expect("hello").expect("ok");
        ws.send(Message::Text(
            serde_json::to_string(&GatewayInboundMessage::HelloAck {
                protocol_version: ProtocolV10,
            })
            .unwrap()
            .into(),
        ))
        .await
        .expect("ack");
        ws.send(Message::Text(
            serde_json::to_string(&GatewayInboundMessage::Command(
                GatewayCommand::DeviceSession {
                    request_id: "req-bind".into(),
                    thread_id: "t-slow".into(),
                    deadline_at: Utc::now() + ChronoDuration::minutes(1),
                    computer_session_id: "session-slow-cleanup-01".into(),
                    computer_session_generation: 1,
                    working_directory: project_for_server.display().to_string(),
                    expires_at: Utc::now() + ChronoDuration::minutes(10),
                },
            ))
            .unwrap()
            .into(),
        ))
        .await
        .expect("send bind");
        let _ = ws.next().await.expect("bind result").expect("ok");
        // Send a slow terminal command, then drop the socket immediately while the command is running
        ws.send(Message::Text(
            serde_json::to_string(&GatewayInboundMessage::Command(
                GatewayCommand::ProjectOperation {
                    request_id: "req-slow-cmd".into(),
                    thread_id: "t-slow".into(),
                    deadline_at: Utc::now() + ChronoDuration::minutes(1),
                    computer_session_id: Some("session-slow-cleanup-01".into()),
                    operation: ProjectOperation::Terminal(TerminalRequest::TerminalExec {
                        terminal_id: None,
                        shell: TerminalShell::Powershell,
                        command: "Start-Sleep -Seconds 2".into(),
                        cwd: None,
                        environment: std::collections::BTreeMap::new(),
                        timeout_seconds: 10,
                        cancel_previous: false,
                    }),
                },
            ))
            .unwrap()
            .into(),
        ))
        .await
        .expect("send slow cmd");
        tokio::time::sleep(Duration::from_millis(150)).await;
        let _ = ws.close(None).await;
    });

    let (ws_client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
        .await
        .expect("connect");
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let client_task = tokio::spawn(async move {
        let dispatcher = std::sync::Arc::new(LocalProjectDispatcher::new());
        cdr_remote_agent::bridge::serve_socket_until_with_status(
            ws_client,
            "device-stop-check",
            dispatcher,
            1,
            shutdown_rx,
            Some(&status_for_client),
        )
        .await
    });

    let ready_rec = wait_for_status(&status_file, |s| s.ready && s.hello_ack).await;
    assert_eq!(ready_rec.generation, 1);
    server_task.await.expect("server closed socket");

    // Even if a heartbeat fires right after socket close, status must already be disconnected (!ready)
    status.record_heartbeat();
    let disconnected_rec = wait_for_status(&status_file, |s| !s.ready && !s.hello_ack).await;
    assert!(!disconnected_rec.ready);
    let _ = client_task.await.expect("client task");

    // 4. Verify mark_shutting_down() latches shutdown so a late hello_ack cannot re-publish ready=true
    status.mark_shutting_down();
    assert!(status.is_shutting_down());
    assert!(!status.is_connected());
    // Simulate a late hello_ack arriving after shutdown was signaled on the status tracker
    let (ws_late, _) = {
        let listener2 = TcpListener::bind("127.0.0.1:0").await.expect("bind 2");
        let addr2 = listener2.local_addr().expect("addr 2");
        tokio::spawn(async move {
            let (s, _) = listener2.accept().await.expect("accept 2");
            let mut ws = accept_async(s).await.expect("ws 2");
            let _ = ws.next().await;
            let _ = ws
                .send(Message::Text(
                    serde_json::to_string(&GatewayInboundMessage::HelloAck {
                        protocol_version: ProtocolV10,
                    })
                    .unwrap()
                    .into(),
                ))
                .await;
            let _ = ws.close(None).await;
        });
        tokio_tungstenite::connect_async(format!("ws://{addr2}"))
            .await
            .expect("connect 2")
    };
    let (_tx2, rx2) = tokio::sync::watch::channel(false);
    let _ = cdr_remote_agent::bridge::serve_socket_until_with_status(
        ws_late,
        "device-stop-check",
        std::sync::Arc::new(LocalProjectDispatcher::new()),
        2,
        rx2,
        Some(&status),
    )
    .await;
    assert!(
        !status.is_connected(),
        "Shutting-down status must remain latched against late hello_ack"
    );
    let after_late_ack: ExternalMcpStatusRecord =
        serde_json::from_slice(&fs::read(&status_file).expect("read status")).expect("parse");
    assert!(
        !after_late_ack.ready && !after_late_ack.hello_ack,
        "Late hello_ack after mark_shutting_down must never publish ready=true"
    );
}

#[tokio::test]
async fn standalone_mcp_distinguishes_process_heartbeat_from_bridge_liveness_and_recovers_on_new_generation()
 {
    let temp = tempfile::tempdir().expect("tempdir");
    let data_dir = temp.path().join("bot_data");
    fs::create_dir_all(&data_dir).expect("create bot_data");
    let status_file = data_dir.join("remote_mcp_status.json");
    let stop_file = data_dir.join("remote_mcp.stop");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("addr");
    let env_file = temp.path().join(".env");
    fs::write(
        &env_file,
        format!(
            "CODEX_REMOTE_MCP_ENABLED=1\n\
             CODEX_REMOTE_MCP_OWNERSHIP=external\n\
             CODEX_REMOTE_MCP_BRIDGE_URL=ws://{address}\n\
             CODEX_REMOTE_MCP_DEVICE_ID=device-liveness\n\
             CODEX_REMOTE_MCP_DEVICE_TOKEN=secret-token-value\n"
        ),
    )
    .expect("write env");

    let (enter_blackhole_tx, mut enter_blackhole_rx) = tokio::sync::oneshot::channel::<()>();
    let (blackhole_active_tx, blackhole_active_rx) = tokio::sync::oneshot::channel::<()>();
    let (release_blackhole_tx, release_blackhole_rx) = tokio::sync::oneshot::channel::<()>();

    let server = tokio::spawn(async move {
        // Generation 1: Healthy idle connection responding to WebSocket Ping with Pong,
        // then transitions into a blackholed peer (stops reading/responding without closing socket).
        let (stream1, _) = listener.accept().await.expect("accept gen 1");
        let mut ws1 = accept_async(stream1).await.expect("ws gen 1");
        let hello1 = ws1.next().await.expect("hello 1").expect("msg 1");
        parse_bridge_message(hello1.to_text().expect("text")).expect("parse hello 1");
        ws1.send(Message::Text(
            serde_json::to_string(&GatewayInboundMessage::HelloAck {
                protocol_version: ProtocolV10,
            })
            .unwrap()
            .into(),
        ))
        .await
        .expect("send ack 1");

        loop {
            tokio::select! {
                _ = &mut enter_blackhole_rx => {
                    break;
                }
                msg = ws1.next() => {
                    match msg {
                        Some(Ok(Message::Ping(payload))) => {
                            let _ = ws1.send(Message::Pong(payload)).await;
                            let _ = ws1.flush().await;
                        }
                        Some(Ok(_)) => {
                            let _ = ws1.flush().await;
                        }
                        _ => break,
                    }
                }
            }
        }
        let _ = blackhole_active_tx.send(());

        // Accept Generation 2 while keeping ws1 open (never closed by peer!) to prove
        // client liveness timeout disconnected and reconnected with a new generation.
        let (stream2, _) = listener.accept().await.expect("accept gen 2");
        let mut ws2 = accept_async(stream2).await.expect("ws gen 2");
        let hello2 = ws2.next().await.expect("hello 2").expect("msg 2");
        parse_bridge_message(hello2.to_text().expect("text")).expect("parse hello 2");
        ws2.send(Message::Text(
            serde_json::to_string(&GatewayInboundMessage::HelloAck {
                protocol_version: ProtocolV10,
            })
            .unwrap()
            .into(),
        ))
        .await
        .expect("send ack 2");

        while let Some(Ok(msg)) = ws2.next().await {
            if let Message::Ping(payload) = msg {
                let _ = ws2.send(Message::Pong(payload)).await;
                let _ = ws2.flush().await;
            }
        }
        let _ = release_blackhole_rx.await;
        drop(ws1);
    });

    let mut child = OwnedChildGuard::new(
        clean_mcp_command()
            .arg("--env")
            .arg(&env_file)
            .current_dir(temp.path())
            .env("CODEX_REMOTE_MCP_RECONNECT_SECONDS", "1")
            .env("CDR_REMOTE_MCP_PING_INTERVAL_MS", "200")
            .env("CDR_REMOTE_MCP_LIVENESS_TIMEOUT_MS", "900")
            .env("CDR_REMOTE_MCP_LIVENESS_STALE_MS", "900")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn standalone mcp"),
    );

    // 1. Wait for generation 1 ready
    let initial = wait_for_status(&status_file, |s| {
        s.ready && s.hello_ack && s.generation == 1
    })
    .await;
    let initial_verified = initial
        .connection_verified_at_ms
        .expect("connection_verified_at_ms present when connected");
    assert_eq!(initial.updated_at_ms, initial_verified);

    // 2. Healthy idle connection stays ready past the 900ms stale window via WebSocket Ping/Pong
    tokio::time::sleep(Duration::from_millis(1_150)).await;
    let healthy_idle: ExternalMcpStatusRecord =
        serde_json::from_slice(&fs::read(&status_file).expect("read idle status"))
            .expect("parse idle status");
    assert!(
        healthy_idle.ready && healthy_idle.hello_ack && healthy_idle.generation == 1,
        "Healthy idle connection must stay ready via Ping/Pong: {healthy_idle:?}"
    );
    let refreshed_verified = healthy_idle
        .connection_verified_at_ms
        .expect("connection_verified_at_ms present on idle connection");
    assert!(
        refreshed_verified > initial_verified,
        "WebSocket Pong must refresh connection_verified_at_ms: initial={initial_verified}, refreshed={refreshed_verified}"
    );

    // 3. Enter blackhole (peer stops reading/responding without closing socket while process heartbeat continues)
    let _ = enter_blackhole_tx.send(());
    blackhole_active_rx.await.expect("blackhole active");

    // Wait for status to observe a process heartbeat AFTER the last bridge Pong so process vs connection timestamps diverge
    let diverged = wait_for_status(&status_file, |s| {
        s.heartbeat_at_ms > s.updated_at_ms || (!s.ready && !s.hello_ack)
    })
    .await;
    if diverged.ready {
        assert!(
            diverged.heartbeat_at_ms > diverged.updated_at_ms,
            "Process heartbeat_at_ms must advance without refreshing connection updated_at_ms: {diverged:?}"
        );
    }

    // Within bounded liveness window, readiness must be invalidated even though socket was never closed by peer
    let invalidated = wait_for_status(&status_file, |s| !s.ready && !s.hello_ack).await;
    assert!(!invalidated.ready);
    assert!(!invalidated.hello_ack);

    // 4. Client automatically reconnects on a new generation (generation == 2) and restores readiness
    let recovered = wait_for_status(&status_file, |s| {
        s.ready && s.hello_ack && s.generation >= 2
    })
    .await;
    assert_eq!(recovered.process_identity, initial.process_identity);
    assert!(
        recovered.connection_verified_at_ms.unwrap_or(0) > refreshed_verified,
        "Reconnected generation must have fresh connection_verified_at_ms"
    );

    // 5. Stop cleanly
    fs::write(&stop_file, "stop\n").expect("write stop");
    let exit_status = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = child.inner_mut().try_wait().expect("try_wait") {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("child exited");
    assert!(exit_status.success());
    let _ = release_blackhole_tx.send(());
    server.await.expect("server task");
}

async fn wait_for_status(
    path: &std::path::Path,
    predicate: impl Fn(&ExternalMcpStatusRecord) -> bool,
) -> ExternalMcpStatusRecord {
    tokio::time::timeout(Duration::from_secs(5), async {
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
    .expect("timed out waiting for status predicate")
}
