use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use cdr_remote_agent::config::load_remote_mcp_config;
use cdr_remote_agent::runner::{run_remote_agent, run_remote_agent_with_status};
use cdr_remote_agent::status::RemoteAgentStatus;
use cdr_remote_protocol::message::{GatewayInboundMessage, ProtocolV10, parse_bridge_message};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn remote_agent_reconnects_and_stops_on_shutdown() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let server_count = Arc::clone(&accepted);
    let server = tokio::spawn(async move {
        while server_count.load(Ordering::SeqCst) < 2 {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let hello = socket.next().await.unwrap().unwrap();
            parse_bridge_message(hello.to_text().unwrap()).unwrap();
            socket
                .send(Message::Text(
                    serde_json::to_string(&GatewayInboundMessage::HelloAck {
                        protocol_version: ProtocolV10,
                    })
                    .unwrap()
                    .into(),
                ))
                .await
                .unwrap();
            server_count.fetch_add(1, Ordering::SeqCst);
            socket.close(None).await.unwrap();
        }
    });
    let values = HashMap::from([
        ("CODEX_REMOTE_MCP_ENABLED".into(), "1".into()),
        (
            "CODEX_REMOTE_MCP_BRIDGE_URL".into(),
            format!("ws://{address}"),
        ),
        ("CODEX_REMOTE_MCP_DEVICE_ID".into(), "device-test".into()),
        (
            "CODEX_REMOTE_MCP_DEVICE_TOKEN".into(),
            "secret-test-token".into(),
        ),
    ]);
    let mut config = load_remote_mcp_config(&values).unwrap().unwrap();
    config.reconnect_delay_seconds = 0;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let agent = tokio::spawn(run_remote_agent(config, shutdown_rx));

    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("agent reconnected")
        .unwrap();
    shutdown_tx.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(2), agent)
        .await
        .expect("agent stopped")
        .unwrap();

    assert_eq!(accepted.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn remote_agent_status_is_connected_only_after_gateway_hello_ack() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let _ = socket.next().await.unwrap().unwrap();
        socket
            .send(Message::Text(
                serde_json::to_string(&GatewayInboundMessage::HelloAck {
                    protocol_version: ProtocolV10,
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        ready_tx.send(()).unwrap();
        let _ = socket.next().await;
    });
    let values = HashMap::from([
        ("CODEX_REMOTE_MCP_ENABLED".into(), "1".into()),
        (
            "CODEX_REMOTE_MCP_BRIDGE_URL".into(),
            format!("ws://{address}"),
        ),
        ("CODEX_REMOTE_MCP_DEVICE_ID".into(), "device-test".into()),
        (
            "CODEX_REMOTE_MCP_DEVICE_TOKEN".into(),
            "secret-test-token".into(),
        ),
    ]);
    let config = load_remote_mcp_config(&values).unwrap().unwrap();
    let status = RemoteAgentStatus::default();
    let observed = status.clone();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let agent = tokio::spawn(run_remote_agent_with_status(config, shutdown_rx, status));

    ready_rx.await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !observed.is_connected() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(observed.generation(), 1);
    shutdown_tx.send(true).unwrap();
    agent.await.unwrap();
    server.await.unwrap();
    assert!(!observed.is_connected());
}
