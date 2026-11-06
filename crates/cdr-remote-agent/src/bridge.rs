use std::sync::Arc;

use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Error as WebSocketError;

use cdr_remote_protocol::message::{
    BridgeInboundMessage, BridgeResult, GatewayInboundMessage, ProtocolError, ProtocolV10,
};

use crate::dispatcher::LocalProjectDispatcher;
use crate::status::RemoteAgentStatus;
use workers::BoundedWorkers;

mod client;
mod io;
pub mod workers;
pub use client::{connect_and_serve, connect_and_serve_until, connect_and_serve_until_with_status};
use io::{GatewayFrame, receive_gateway, receive_gateway_frame, send_json, send_ping};

const MAX_WORKERS: usize = 4;
const MAX_PENDING: usize = 16;
const DEFAULT_BRIDGE_PING_INTERVAL_MS: u64 = 1_500;
const DEFAULT_BRIDGE_LIVENESS_TIMEOUT_MS: u64 = 4_500;

fn bridge_ping_interval() -> std::time::Duration {
    let ms = std::env::var("CDR_REMOTE_MCP_PING_INTERVAL_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&v| (100..=30_000).contains(&v))
        .unwrap_or(DEFAULT_BRIDGE_PING_INTERVAL_MS);
    std::time::Duration::from_millis(ms)
}

fn bridge_liveness_timeout() -> std::time::Duration {
    let ms = std::env::var("CDR_REMOTE_MCP_LIVENESS_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&v| (200..=60_000).contains(&v))
        .unwrap_or(DEFAULT_BRIDGE_LIVENESS_TIMEOUT_MS);
    std::time::Duration::from_millis(ms)
}

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("the gateway sent an unsupported binary message")]
    UnsupportedBinary,
    #[error("the gateway closed before acknowledging the bridge hello")]
    ClosedBeforeHello,
    #[error("the gateway did not acknowledge the bridge hello")]
    MissingHelloAck,
    #[error("the gateway sent a duplicate hello")]
    DuplicateHello,
    #[error("the bridge connection timed out")]
    ConnectTimeout,
    #[error("the bridge peer stopped responding within the liveness deadline")]
    LivenessTimeout,
    #[error("the device authorization token is not a valid HTTP header value")]
    InvalidAuthorization,
    #[error("local bridge generation error: {0}")]
    Generation(String),
    #[error("local bridge worker failed: {0}")]
    Worker(String),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    WebSocket(#[from] WebSocketError),
}

pub async fn serve_socket<S>(
    socket: WebSocketStream<S>,
    device_id: &str,
    dispatcher: Arc<LocalProjectDispatcher>,
    generation: u64,
) -> Result<(), BridgeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (_keep_alive, shutdown) = watch::channel(false);
    serve_socket_until(socket, device_id, dispatcher, generation, shutdown).await
}

pub async fn serve_socket_until<S>(
    socket: WebSocketStream<S>,
    device_id: &str,
    dispatcher: Arc<LocalProjectDispatcher>,
    generation: u64,
    shutdown: watch::Receiver<bool>,
) -> Result<(), BridgeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    serve_socket_until_with_status(socket, device_id, dispatcher, generation, shutdown, None).await
}

pub async fn serve_socket_until_with_status<S>(
    mut socket: WebSocketStream<S>,
    device_id: &str,
    dispatcher: Arc<LocalProjectDispatcher>,
    generation: u64,
    mut shutdown: watch::Receiver<bool>,
    status: Option<&RemoteAgentStatus>,
) -> Result<(), BridgeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let result = serve_inner(
        &mut socket,
        device_id,
        &dispatcher,
        generation,
        &mut shutdown,
        status,
    )
    .await;
    dispatcher
        .retire_sessions()
        .await
        .map_err(BridgeError::Generation)?;
    result
}

async fn serve_inner<S>(
    socket: &mut WebSocketStream<S>,
    device_id: &str,
    dispatcher: &Arc<LocalProjectDispatcher>,
    generation: u64,
    shutdown: &mut watch::Receiver<bool>,
    status: Option<&RemoteAgentStatus>,
) -> Result<(), BridgeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    dispatcher
        .begin_connection(generation)
        .await
        .map_err(BridgeError::Generation)?;
    send_json(
        socket,
        &BridgeInboundMessage::Hello {
            protocol_version: ProtocolV10,
            device_id: device_id.to_owned(),
        },
    )
    .await?;
    match receive_gateway(socket, true).await? {
        Some(GatewayInboundMessage::HelloAck { .. }) => {}
        Some(_) => return Err(BridgeError::MissingHelloAck),
        None => return Err(BridgeError::ClosedBeforeHello),
    }
    if let Some(status) = status {
        status.set_connected(generation);
    }
    let (cancel, cancelled) = watch::channel(false);
    let mut workers = BoundedWorkers::new(MAX_WORKERS, MAX_PENDING);
    let mut result = serve_commands(
        socket,
        dispatcher,
        generation,
        cancelled,
        &mut workers,
        shutdown,
        status,
    )
    .await;
    if let Some(status) = status {
        status.set_disconnected();
    }
    let _ = cancel.send(true);
    while let Some(completed) = workers.join_next().await {
        if let Err(error) = completed
            && result.is_ok()
        {
            result = Err(BridgeError::Worker(error.to_string()));
        }
    }
    result
}

async fn serve_commands<S>(
    socket: &mut WebSocketStream<S>,
    dispatcher: &Arc<LocalProjectDispatcher>,
    generation: u64,
    cancelled: watch::Receiver<bool>,
    workers: &mut BoundedWorkers<BridgeResult>,
    shutdown: &mut watch::Receiver<bool>,
    status: Option<&RemoteAgentStatus>,
) -> Result<(), BridgeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let ping_every = bridge_ping_interval();
    let liveness_timeout = bridge_liveness_timeout();
    let mut ping_interval =
        tokio::time::interval_at(tokio::time::Instant::now() + ping_every, ping_every);
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_inbound_at = tokio::time::Instant::now();
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
            completed = workers.join_next(), if !workers.is_empty() => {
                let result = completed
                    .expect("a worker was present")
                    .map_err(|error| BridgeError::Worker(error.to_string()))?;
                send_json(socket, &BridgeInboundMessage::Result(result)).await?;
            }
            () = tokio::time::sleep_until(last_inbound_at + liveness_timeout) => {
                if let Some(status) = status {
                    status.set_disconnected();
                }
                return Err(BridgeError::LivenessTimeout);
            }
            _ = ping_interval.tick() => {
                send_ping(socket).await?;
            }
            frame = receive_gateway_frame(socket, false) => {
                match frame? {
                    GatewayFrame::Closed => return Ok(()),
                    GatewayFrame::ControlAck => {
                        last_inbound_at = tokio::time::Instant::now();
                        if let Some(status) = status {
                            status.record_bridge_inbound(generation);
                        }
                    }
                    GatewayFrame::Message(message) => {
                        last_inbound_at = tokio::time::Instant::now();
                        if let Some(status) = status {
                            status.record_bridge_inbound(generation);
                        }
                        match message {
                            GatewayInboundMessage::HelloAck { .. } => {
                                return Err(BridgeError::DuplicateHello);
                            }
                            GatewayInboundMessage::ProjectAck { .. } => {}
                            GatewayInboundMessage::Command(command) => {
                                submit_command(
                                    socket,
                                    dispatcher,
                                    generation,
                                    cancelled.clone(),
                                    workers,
                                    command,
                                )
                                .await?;
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn submit_command<S>(
    socket: &mut WebSocketStream<S>,
    dispatcher: &Arc<LocalProjectDispatcher>,
    generation: u64,
    cancelled: watch::Receiver<bool>,
    workers: &mut BoundedWorkers<BridgeResult>,
    command: cdr_remote_protocol::message::GatewayCommand,
) -> Result<(), BridgeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let request_id = command.request_id().to_owned();
    let dispatcher = Arc::clone(dispatcher);
    if workers
        .submit(async move {
            dispatcher
                .execute_cancellable(command, Some(generation), cancelled)
                .await
        })
        .is_err()
    {
        let result = BridgeResult::OperationError {
            request_id,
            error_code: "bridge_busy".into(),
            message: "The local bridge is busy. Retry this request shortly.".into(),
        };
        send_json(socket, &BridgeInboundMessage::Result(result)).await?;
    }
    Ok(())
}
