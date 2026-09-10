use std::time::Duration;

use cdr_mcp_server::bridge_http::{DeviceCredential, DeviceCredentialRegistry, bridge_router};
use cdr_mcp_server::broker::BridgeBroker;
use cdr_remote_protocol::message::{
    BridgeInboundMessage, BridgeResult, GatewayInboundMessage, ProtocolV10, parse_gateway_message,
};
use chrono::{TimeDelta, Utc};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{Error as WebSocketError, Message};

#[tokio::test]
async fn authenticated_websocket_registers_and_routes_a_project() {
    let broker = BridgeBroker::new(Duration::from_secs(2));
    let credentials = DeviceCredentialRegistry::new(vec![
        DeviceCredential::new("windows-a", "a-device-token-with-at-least-32-bytes")
            .expect("valid credential"),
    ])
    .expect("valid registry");
    let app = bridge_router(credentials, broker.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind server");
    let address = listener.local_addr().expect("server address");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve bridge") });
    let mut request = format!("ws://{address}/bridge")
        .into_client_request()
        .expect("websocket request");
    request.headers_mut().insert(
        "authorization",
        HeaderValue::from_static("Bearer a-device-token-with-at-least-32-bytes"),
    );
    let (mut socket, _) = connect_async(request).await.expect("connect bridge");
    socket
        .send(Message::Text(
            serde_json::to_string(&BridgeInboundMessage::Hello {
                protocol_version: ProtocolV10,
                device_id: "windows-a".into(),
            })
            .expect("serialize hello")
            .into(),
        ))
        .await
        .expect("send hello");
    assert!(matches!(
        receive(&mut socket).await,
        GatewayInboundMessage::HelloAck { .. }
    ));
    socket
        .send(Message::Text(
            serde_json::json!({
                "type":"project_upsert",
                "project_scope":"project-scope-1234",
                "binding_id":"binding-12345678",
                "thread_id":"thread-a",
                "project_name":"Sample",
                "expires_at": Utc::now() + TimeDelta::minutes(5),
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("register project");
    assert!(matches!(
        receive(&mut socket).await,
        GatewayInboundMessage::ProjectAck { .. }
    ));

    let selection = tokio::spawn(async move {
        broker
            .select_project("chat-a", "principal-a", "project-scope-1234")
            .await
    });
    let command = receive(&mut socket).await;
    let GatewayInboundMessage::Command(command) = command else {
        panic!("expected project session command")
    };
    let request_id = command.request_id().to_owned();
    socket
        .send(Message::Text(
            serde_json::to_string(&BridgeInboundMessage::Result(
                BridgeResult::ProjectSessionResult { request_id },
            ))
            .expect("serialize result")
            .into(),
        ))
        .await
        .expect("complete selection");
    assert_eq!(
        selection
            .await
            .expect("selection task")
            .expect("selection")
            .project_name,
        "Sample"
    );
}

#[tokio::test]
async fn websocket_upgrade_without_a_device_token_is_rejected() {
    let broker = BridgeBroker::new(Duration::from_secs(2));
    let credentials = DeviceCredentialRegistry::new(vec![
        DeviceCredential::new("windows-a", "a-device-token-with-at-least-32-bytes")
            .expect("valid credential"),
    ])
    .expect("valid registry");
    let app = bridge_router(credentials, broker);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind server");
    let address = listener.local_addr().expect("server address");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve bridge") });
    let error = connect_async(format!("ws://{address}/bridge"))
        .await
        .expect_err("missing token must reject upgrade");
    assert!(matches!(
        error,
        WebSocketError::Http(response)
            if response.status() == tokio_tungstenite::tungstenite::http::StatusCode::UNAUTHORIZED
    ));
}

async fn receive(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> GatewayInboundMessage {
    let Message::Text(text) = socket
        .next()
        .await
        .expect("websocket message")
        .expect("valid websocket frame")
    else {
        panic!("expected text frame")
    };
    parse_gateway_message(text.as_str()).expect("valid gateway message")
}
