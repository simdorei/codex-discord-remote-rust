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
use io::{receive_gateway, send_json};

const MAX_WORKERS: usize = 4;
const MAX_PENDING: usize = 16;

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

async fn serve_socket_until_with_status<S>(
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
    )
    .await;
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
) -> Result<(), BridgeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
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
            message = receive_gateway(socket, false) => {
                let Some(message) = message? else {
                    return Ok(());
                };
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
