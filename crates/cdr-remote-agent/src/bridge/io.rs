use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use cdr_remote_protocol::message::{GatewayInboundMessage, parse_gateway_message};

use super::BridgeError;

pub(super) async fn receive_gateway<S>(
    socket: &mut WebSocketStream<S>,
    waiting_for_hello: bool,
) -> Result<Option<GatewayInboundMessage>, BridgeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        let Some(message) = socket.next().await else {
            return Ok(None);
        };
        match message? {
            Message::Text(text) => return Ok(Some(parse_gateway_message(text.as_str())?)),
            Message::Binary(_) => return Err(BridgeError::UnsupportedBinary),
            Message::Ping(payload) => socket.send(Message::Pong(payload)).await?,
            Message::Pong(_) | Message::Frame(_) => {}
            Message::Close(_) if waiting_for_hello => return Err(BridgeError::ClosedBeforeHello),
            Message::Close(_) => return Ok(None),
        }
    }
}

pub(super) async fn send_json<S, T>(
    socket: &mut WebSocketStream<S>,
    value: &T,
) -> Result<(), BridgeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    T: Serialize,
{
    socket
        .send(Message::Text(serde_json::to_string(value)?.into()))
        .await?;
    Ok(())
}
