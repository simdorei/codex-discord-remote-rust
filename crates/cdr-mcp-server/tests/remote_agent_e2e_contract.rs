use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use cdr_mcp_server::bridge_http::{DeviceCredential, DeviceCredentialRegistry, bridge_router};
use cdr_mcp_server::broker::BridgeBroker;
use cdr_remote_agent::bridge::connect_and_serve_until;
use cdr_remote_agent::config::load_remote_mcp_config;
use cdr_remote_agent::dispatcher::LocalProjectDispatcher;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn real_rust_agent_serves_a_file_through_the_rust_gateway() {
    let temp = tempfile::tempdir().expect("temporary project");
    std::fs::write(temp.path().join("hello.txt"), "hello from rust").expect("seed project file");
    let broker = BridgeBroker::new(Duration::from_secs(3));
    let credentials = DeviceCredentialRegistry::new(vec![
        DeviceCredential::new("windows-a", "a-device-token-with-at-least-32-bytes")
            .expect("credential"),
    ])
    .expect("registry");
    let app = bridge_router(credentials, broker.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gateway");
    let address = listener.local_addr().expect("gateway address");
    let server_shutdown = CancellationToken::new();
    let server_token = server_shutdown.clone();
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(server_token.cancelled_owned())
            .await
            .expect("serve gateway");
    });
    let config = load_remote_mcp_config(&HashMap::from([
        ("CODEX_REMOTE_MCP_ENABLED".into(), "1".into()),
        (
            "CODEX_REMOTE_MCP_BRIDGE_URL".into(),
            format!("ws://{address}/bridge"),
        ),
        ("CODEX_REMOTE_MCP_DEVICE_ID".into(), "windows-a".into()),
        (
            "CODEX_REMOTE_MCP_DEVICE_TOKEN".into(),
            "a-device-token-with-at-least-32-bytes".into(),
        ),
    ]))
    .expect("agent config")
    .expect("enabled agent");
    let (stop_agent, stop_rx) = watch::channel(false);
    let agent = tokio::spawn(async move {
        connect_and_serve_until(&config, Arc::new(LocalProjectDispatcher::new()), 1, stop_rx).await
    });
    wait_for_device(&broker).await;
    broker
        .select_device(
            "chat-a",
            "principal-a",
            "windows-a",
            &temp.path().display().to_string(),
        )
        .await
        .expect("select local project folder");
    let read = broker
        .read_file(
            "chat-a",
            "principal-a",
            "request-1234567890".into(),
            "hello.txt".into(),
            1,
            20,
            CancellationToken::new(),
        )
        .await
        .expect("read through real agent");
    assert_eq!(read.content, "hello from rust");
    stop_agent.send_replace(true);
    agent.await.expect("agent task").expect("agent shutdown");
    server_shutdown.cancel();
}

async fn wait_for_device(broker: &BridgeBroker) {
    for _ in 0..50 {
        if !broker.list_devices().await.is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("Rust agent did not connect to the gateway");
}
