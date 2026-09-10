use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use cdr_remote_agent::config::load_remote_mcp_config;
use cdr_remote_agent::dispatcher::LocalProjectDispatcher;
use cdr_remote_agent::restart_handoff::{
    HandoffProtector, RestartHandoffError, RestartHandoffRuntime,
};
use cdr_remote_agent::runner::{RemoteAgentControl, run_remote_agent_managed};
use cdr_remote_agent::status::RemoteAgentStatus;
use cdr_remote_protocol::message::{
    GatewayCommand, GatewayInboundMessage, ProtocolV10, parse_bridge_message,
};
use chrono::{Duration as ChronoDuration, Utc};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

struct XorProtector;

impl HandoffProtector for XorProtector {
    fn protect(&self, payload: &[u8]) -> Result<Vec<u8>, RestartHandoffError> {
        Ok(payload.iter().map(|value| value ^ 0xA5).collect())
    }

    fn unprotect(&self, payload: &[u8]) -> Result<Vec<u8>, RestartHandoffError> {
        self.protect(payload)
    }
}

#[tokio::test]
async fn managed_runner_prepares_active_binding_only_for_requested_restart() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    fs::create_dir(&project).unwrap();
    let handoff = directory.path().join("handoff.json");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let expires_at = Utc::now() + ChronoDuration::minutes(10);
    let project_for_server = project.clone();
    let (bound_tx, bound_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let hello = socket.next().await.unwrap().unwrap();
        parse_bridge_message(hello.to_text().unwrap()).unwrap();
        socket
            .send(json_message(&GatewayInboundMessage::HelloAck {
                protocol_version: ProtocolV10,
            }))
            .await
            .unwrap();
        socket
            .send(json_message(&GatewayInboundMessage::Command(
                GatewayCommand::DeviceSession {
                    request_id: "request-00000001".into(),
                    thread_id: "thread-a".into(),
                    deadline_at: Utc::now() + ChronoDuration::minutes(1),
                    computer_session_id: "session-00000001".into(),
                    computer_session_generation: 1,
                    working_directory: project_for_server.display().to_string(),
                    expires_at,
                },
            )))
            .await
            .unwrap();
        let _result = socket.next().await.unwrap().unwrap();
        bound_tx.send(()).unwrap();
        let _ = socket.next().await;
    });
    let config = config(&address.to_string());
    let control = RemoteAgentControl::default();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let runtime = RestartHandoffRuntime::new(&handoff, Arc::new(XorProtector), false);
    let agent_control = control.clone();
    let agent_config = config.clone();
    let agent = tokio::spawn(async move {
        run_remote_agent_managed(
            agent_config,
            shutdown_rx,
            RemoteAgentStatus::default(),
            runtime,
            agent_control,
        )
        .await
    });

    tokio::time::timeout(Duration::from_secs(5), bound_rx)
        .await
        .unwrap()
        .unwrap();
    control.request_restart_handoff();
    shutdown_tx.send(true).unwrap();
    agent.await.unwrap().unwrap();
    server.await.unwrap();

    let replacement = LocalProjectDispatcher::new();
    let resume = RestartHandoffRuntime::new(&handoff, Arc::new(XorProtector), true);
    assert_eq!(
        resume
            .restore(&replacement, &config, Utc::now())
            .await
            .unwrap(),
        1
    );
    let projects = replacement.restart_projects(Utc::now()).await;
    assert_eq!(projects[0].thread_id, "thread-a");
    assert_eq!(projects[0].root, project.canonicalize().unwrap());
}

fn json_message(message: &GatewayInboundMessage) -> Message {
    Message::Text(serde_json::to_string(message).unwrap().into())
}

fn config(address: &str) -> cdr_remote_agent::config::RemoteMcpConfig {
    load_remote_mcp_config(&HashMap::from([
        ("CODEX_REMOTE_MCP_ENABLED".into(), "1".into()),
        (
            "CODEX_REMOTE_MCP_BRIDGE_URL".into(),
            format!("ws://{address}"),
        ),
        ("CODEX_REMOTE_MCP_DEVICE_ID".into(), "device-a".into()),
        ("CODEX_REMOTE_MCP_DEVICE_TOKEN".into(), "secret".into()),
    ]))
    .unwrap()
    .unwrap()
}
