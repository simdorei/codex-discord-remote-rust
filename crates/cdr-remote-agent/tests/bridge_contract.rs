use std::fs;
use std::sync::Arc;

use cdr_remote_agent::bridge::{BridgeError, serve_socket};
use cdr_remote_agent::dispatcher::LocalProjectDispatcher;
use cdr_remote_protocol::message::{
    BridgeInboundMessage, BridgeResult, GatewayCommand, GatewayInboundMessage, ProtocolV10,
    parse_bridge_message,
};
use chrono::{Duration, Utc};
use futures_util::{SinkExt, StreamExt};
use tokio::io::duplex;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::Role;

async fn send_gateway(
    socket: &mut WebSocketStream<tokio::io::DuplexStream>,
    message: &GatewayInboundMessage,
) {
    socket
        .send(Message::Text(
            serde_json::to_string(message).expect("serialize").into(),
        ))
        .await
        .expect("send");
}

async fn receive_bridge(
    socket: &mut WebSocketStream<tokio::io::DuplexStream>,
) -> BridgeInboundMessage {
    let message = socket.next().await.expect("message").expect("websocket");
    parse_bridge_message(message.to_text().expect("text")).expect("protocol")
}

#[tokio::test]
async fn b1_protocol_ten_device_session_file_round_trip_and_close() {
    let directory = tempfile::tempdir().expect("tempdir");
    fs::write(directory.path().join("notes.txt"), "first").expect("fixture");
    let (local, remote) = duplex(128 * 1024);
    let local = WebSocketStream::from_raw_socket(local, Role::Client, None).await;
    let mut remote = WebSocketStream::from_raw_socket(remote, Role::Server, None).await;
    let dispatcher = Arc::new(LocalProjectDispatcher::new());
    let task = tokio::spawn(serve_socket(local, "device-a", Arc::clone(&dispatcher), 7));

    assert!(matches!(
        receive_bridge(&mut remote).await,
        BridgeInboundMessage::Hello { device_id, .. } if device_id == "device-a"
    ));
    send_gateway(
        &mut remote,
        &GatewayInboundMessage::HelloAck {
            protocol_version: ProtocolV10,
        },
    )
    .await;
    let session_id = "session-aaaaaaaa";
    send_gateway(
        &mut remote,
        &GatewayInboundMessage::Command(GatewayCommand::DeviceSession {
            request_id: "session".into(),
            thread_id: "thread-a".into(),
            deadline_at: Utc::now() + Duration::minutes(1),
            computer_session_id: session_id.into(),
            computer_session_generation: 1,
            working_directory: directory.path().display().to_string(),
            expires_at: Utc::now() + Duration::minutes(10),
        }),
    )
    .await;
    assert!(matches!(
        receive_bridge(&mut remote).await,
        BridgeInboundMessage::Result(BridgeResult::ProjectSessionResult { .. })
    ));

    send_gateway(
        &mut remote,
        &GatewayInboundMessage::Command(GatewayCommand::WriteFile {
            request_id: "write".into(),
            thread_id: "thread-a".into(),
            deadline_at: Utc::now() + Duration::minutes(1),
            computer_session_id: Some(session_id.into()),
            path: "created.txt".into(),
            content: "created".into(),
            expected_sha256: None,
        }),
    )
    .await;
    assert!(matches!(
        receive_bridge(&mut remote).await,
        BridgeInboundMessage::Result(BridgeResult::WriteFileResult { ref output, .. })
            if output.created
    ));
    send_gateway(
        &mut remote,
        &GatewayInboundMessage::Command(GatewayCommand::ReadFile {
            request_id: "read".into(),
            thread_id: "thread-a".into(),
            deadline_at: Utc::now() + Duration::minutes(1),
            computer_session_id: Some(session_id.into()),
            path: "created.txt".into(),
            start_line: 1,
            max_lines: 20,
        }),
    )
    .await;
    assert!(matches!(
        receive_bridge(&mut remote).await,
        BridgeInboundMessage::Result(BridgeResult::ReadFileResult { ref output, .. })
            if output.content == "created"
    ));
    remote.close(None).await.expect("close");
    task.await.expect("task").expect("bridge");
}

#[tokio::test]
async fn b2_binary_or_missing_hello_ack_fails_closed() {
    let (local, remote) = duplex(16 * 1024);
    let local = WebSocketStream::from_raw_socket(local, Role::Client, None).await;
    let mut remote = WebSocketStream::from_raw_socket(remote, Role::Server, None).await;
    let task = tokio::spawn(serve_socket(
        local,
        "device-a",
        Arc::new(LocalProjectDispatcher::new()),
        1,
    ));
    let _ = receive_bridge(&mut remote).await;
    remote
        .send(Message::Binary(vec![1, 2, 3].into()))
        .await
        .expect("binary");
    assert!(matches!(
        task.await.expect("task"),
        Err(BridgeError::UnsupportedBinary)
    ));
}
