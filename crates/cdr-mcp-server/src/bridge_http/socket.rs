use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use cdr_remote_protocol::message::{
    BridgeInboundMessage, GatewayInboundMessage, ProtocolV10, parse_bridge_message,
};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::DeviceCredentialRegistry;
use crate::broker::{BridgeBroker, ProjectRegistration};

const MAX_MESSAGE_BYTES: usize = 12_000_000;

#[derive(Clone)]
pub(super) struct BridgeHttpState {
    pub credentials: DeviceCredentialRegistry,
    pub broker: BridgeBroker,
}

pub(super) async fn upgrade(
    State(state): State<BridgeHttpState>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Response {
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let Some(device_id) = state.credentials.authenticate(authorization) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    websocket
        .max_message_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| serve(socket, state.broker, device_id))
}

async fn serve(socket: WebSocket, broker: BridgeBroker, expected_device_id: String) {
    let (mut sink, mut source) = socket.split();
    let Some(Ok(Message::Text(raw))) = source.next().await else {
        reject(&mut sink, 1002, "hello required").await;
        return;
    };
    let Ok(BridgeInboundMessage::Hello { device_id, .. }) = parse_bridge_message(raw.as_str())
    else {
        reject(&mut sink, 1002, "hello required").await;
        return;
    };
    if !same_identity(&device_id, &expected_device_id) {
        reject(&mut sink, 1008, "device mismatch").await;
        return;
    }
    let mut attachment = broker.attach(&device_id).await;
    let identity = attachment.identity().clone();
    if send_json(
        &mut sink,
        &GatewayInboundMessage::HelloAck {
            protocol_version: ProtocolV10,
        },
    )
    .await
    .is_err()
    {
        broker.detach(&identity).await;
        return;
    }
    loop {
        tokio::select! {
            () = attachment.disconnected.cancelled() => {
                reject(&mut sink, 1012, "bridge replaced").await;
                break;
            }
            command = attachment.commands.recv() => {
                let Some(command) = command else { break };
                if send_json(&mut sink, &GatewayInboundMessage::Command(command)).await.is_err() {
                    break;
                }
            }
            incoming = source.next() => {
                if !handle_incoming(incoming, &mut sink, &broker, &identity).await {
                    break;
                }
            }
        }
    }
    broker.detach(&identity).await;
}

async fn handle_incoming(
    incoming: Option<Result<Message, axum::Error>>,
    sink: &mut SplitSink<WebSocket, Message>,
    broker: &BridgeBroker,
    identity: &crate::broker::BridgeIdentity,
) -> bool {
    match incoming {
        Some(Ok(Message::Text(raw))) => match parse_bridge_message(raw.as_str()) {
            Ok(BridgeInboundMessage::ProjectUpsert {
                project_scope,
                binding_id,
                thread_id,
                project_name,
                expires_at,
            }) => {
                let project = ProjectRegistration {
                    project_scope: project_scope.clone(),
                    binding_id: binding_id.clone(),
                    thread_id,
                    project_name,
                    expires_at,
                };
                if broker.upsert(identity, project.clone()).await.is_err() {
                    reject(sink, 1008, "invalid bridge message").await;
                    return false;
                }
                let acknowledged = send_json(
                    sink,
                    &GatewayInboundMessage::ProjectAck {
                        project_scope,
                        binding_id,
                    },
                )
                .await
                .is_ok();
                if acknowledged {
                    let resume_broker = broker.clone();
                    let resume_identity = identity.clone();
                    tokio::spawn(async move {
                        let _ = resume_broker
                            .resume_project(&resume_identity, &project)
                            .await;
                    });
                }
                acknowledged
            }
            Ok(BridgeInboundMessage::Result(result)) => {
                broker.complete(identity, result).await;
                true
            }
            Ok(BridgeInboundMessage::Hello { .. }) => {
                reject(sink, 1002, "duplicate hello").await;
                false
            }
            Err(_) => {
                reject(sink, 1008, "invalid bridge message").await;
                false
            }
        },
        Some(Ok(Message::Ping(value))) => sink.send(Message::Pong(value)).await.is_ok(),
        Some(Ok(Message::Pong(_))) => true,
        Some(Ok(Message::Close(_)) | Err(_)) | None => false,
        Some(Ok(Message::Binary(_))) => {
            reject(sink, 1003, "text messages required").await;
            false
        }
    }
}

async fn send_json<T: serde::Serialize>(
    sink: &mut SplitSink<WebSocket, Message>,
    value: &T,
) -> Result<(), ()> {
    let payload = serde_json::to_string(value).map_err(|_| ())?;
    sink.send(Message::Text(payload.into()))
        .await
        .map_err(|_| ())
}

async fn reject(sink: &mut SplitSink<WebSocket, Message>, code: u16, reason: &'static str) {
    let _ = sink
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

fn same_identity(left: &str, right: &str) -> bool {
    Sha256::digest(left.as_bytes())
        .ct_eq(&Sha256::digest(right.as_bytes()))
        .into()
}
