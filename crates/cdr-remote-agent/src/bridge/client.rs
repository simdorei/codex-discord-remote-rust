use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use super::{BridgeError, serve_socket, serve_socket_until, serve_socket_until_with_status};
use crate::config::RemoteMcpConfig;
use crate::dispatcher::LocalProjectDispatcher;
use crate::status::RemoteAgentStatus;

const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_MESSAGE_BYTES: usize = 12_000_000;

pub async fn connect_and_serve(
    config: &RemoteMcpConfig,
    dispatcher: Arc<LocalProjectDispatcher>,
    generation: u64,
) -> Result<(), BridgeError> {
    let socket = connect(config).await?;
    serve_socket(socket, &config.device_id, dispatcher, generation).await
}

pub async fn connect_and_serve_until(
    config: &RemoteMcpConfig,
    dispatcher: Arc<LocalProjectDispatcher>,
    generation: u64,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), BridgeError> {
    let socket = tokio::select! {
        result = connect(config) => Some(result?),
        () = wait_for_shutdown(&mut shutdown) => None,
    };
    let Some(socket) = socket else {
        return Ok(());
    };
    serve_socket_until(socket, &config.device_id, dispatcher, generation, shutdown).await
}

pub async fn connect_and_serve_until_with_status(
    config: &RemoteMcpConfig,
    dispatcher: Arc<LocalProjectDispatcher>,
    generation: u64,
    mut shutdown: watch::Receiver<bool>,
    status: &RemoteAgentStatus,
) -> Result<(), BridgeError> {
    let socket = tokio::select! {
        result = connect(config) => Some(result?),
        () = wait_for_shutdown(&mut shutdown) => None,
    };
    let Some(socket) = socket else {
        return Ok(());
    };
    let result = serve_socket_until_with_status(
        socket,
        &config.device_id,
        dispatcher,
        generation,
        shutdown,
        Some(status),
    )
    .await;
    status.set_disconnected();
    result
}

async fn connect(
    config: &RemoteMcpConfig,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    BridgeError,
> {
    let mut request = config.bridge_url.as_str().into_client_request()?;
    let authorization = HeaderValue::from_str(&format!("Bearer {}", config.device_token.expose()))
        .map_err(|_| BridgeError::InvalidAuthorization)?;
    request.headers_mut().insert(AUTHORIZATION, authorization);
    let websocket_config = WebSocketConfig::default().max_message_size(Some(MAX_MESSAGE_BYTES));
    let (socket, _) = tokio::time::timeout(
        OPEN_TIMEOUT,
        connect_async_with_config(request, Some(websocket_config), false),
    )
    .await
    .map_err(|_| BridgeError::ConnectTimeout)??;
    Ok(socket)
}

async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow() || shutdown.changed().await.is_err() {
            return;
        }
    }
}
