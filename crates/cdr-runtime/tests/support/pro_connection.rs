//! Loopback MCP hello only: never connects to a real device or dispatches tools.
use cdr_remote_agent::{
    config::{RemoteMcpConfig, load_remote_mcp_config},
    runner::run_remote_agent_with_status,
    status::RemoteAgentStatus,
};
use futures_util::{SinkExt, StreamExt};
use std::{collections::HashMap, time::Duration};
use tokio::{net::TcpListener, sync::watch, task::JoinHandle};

pub struct Connection {
    pub config: RemoteMcpConfig,
    pub status: RemoteAgentStatus,
    stop: watch::Sender<bool>,
    worker: JoinHandle<()>,
    gateway: JoinHandle<()>,
}

impl Connection {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let gateway = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let _hello = socket.next().await.unwrap().unwrap();
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({"type":"hello_ack","protocol_version":10})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            while socket.next().await.is_some() {}
        });
        let config = load_remote_mcp_config(&HashMap::from([
            ("CODEX_REMOTE_MCP_ENABLED".into(), "1".into()),
            (
                "CODEX_REMOTE_MCP_BRIDGE_URL".into(),
                format!("ws://{address}"),
            ),
            (
                "CODEX_REMOTE_MCP_DEVICE_ID".into(),
                "r10-device-fixture".into(),
            ),
            (
                "CODEX_REMOTE_MCP_DEVICE_TOKEN".into(),
                "public-fixture".into(),
            ),
        ]))
        .unwrap()
        .unwrap();
        let status = RemoteAgentStatus::default();
        let (stop, shutdown) = watch::channel(false);
        let worker = tokio::spawn(run_remote_agent_with_status(
            config.clone(),
            shutdown,
            status.clone(),
        ));
        tokio::time::timeout(Duration::from_secs(3), async {
            while !status.is_connected() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        Self {
            config,
            status,
            stop,
            worker,
            gateway,
        }
    }

    pub async fn close(self) {
        self.stop.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(3), self.worker)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), self.gateway)
            .await
            .unwrap()
            .unwrap();
    }
}
