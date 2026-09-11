//! Native successor to the two-device smoke: no production endpoint or interpreter.
use cdr_mcp_server::{
    bridge_http::{DeviceCredential, DeviceCredentialRegistry, bridge_router},
    broker::BridgeBroker,
};
use cdr_remote_agent::{
    bridge::{BridgeError, connect_and_serve_until},
    config::{RemoteMcpConfig, load_remote_mcp_config},
    dispatcher::LocalProjectDispatcher,
};
use std::{collections::HashMap, path::Path, sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinHandle};
use tokio_util::sync::CancellationToken;

fn token(id: &str) -> String {
    format!("synthetic-device-token-with-at-least-32-bytes-{id}")
}
fn config(address: std::net::SocketAddr, id: &str) -> RemoteMcpConfig {
    load_remote_mcp_config(&HashMap::from([
        ("CODEX_REMOTE_MCP_ENABLED".into(), "1".into()),
        (
            "CODEX_REMOTE_MCP_BRIDGE_URL".into(),
            format!("ws://{address}/bridge"),
        ),
        ("CODEX_REMOTE_MCP_DEVICE_ID".into(), id.into()),
        ("CODEX_REMOTE_MCP_DEVICE_TOKEN".into(), token(id)),
    ]))
    .unwrap()
    .unwrap()
}
fn start(
    config: RemoteMcpConfig,
    dispatcher: Arc<LocalProjectDispatcher>,
) -> (watch::Sender<bool>, JoinHandle<Result<(), BridgeError>>) {
    let (stop, rx) = watch::channel(false);
    let agent =
        tokio::spawn(async move { connect_and_serve_until(&config, dispatcher, 1, rx).await });
    (stop, agent)
}
async fn wait_count(broker: &BridgeBroker, expected: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if broker.list_devices().await.len() == expected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("exact connected-device count was not reached");
}
async fn read(broker: &BridgeBroker, chat: &str, id: &str, root: &Path, request: &str) {
    broker
        .select_device(chat, "principal", id, &root.display().to_string())
        .await
        .unwrap();
    let result = broker
        .read_file(
            chat,
            "principal",
            request.into(),
            "hello.txt".into(),
            1,
            20,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(result.content, "native restart smoke 한글");
}

async fn finish<E: std::fmt::Debug>(task: JoinHandle<Result<(), E>>) {
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn two_real_agents_survive_one_isolated_restart_and_restore_projects() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("hello.txt"), "native restart smoke 한글").unwrap();
    let broker = BridgeBroker::new(Duration::from_secs(3));
    let credentials = DeviceCredentialRegistry::new(vec![
        DeviceCredential::new("device-a", &token("device-a")).unwrap(),
        DeviceCredential::new("device-b", &token("device-b")).unwrap(),
    ])
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let cancel = CancellationToken::new();
    let cleanup = cancel.clone().drop_guard();
    let shutdown = cancel.clone();
    let app = bridge_router(credentials, broker.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
    });
    let dispatcher_a = Arc::new(LocalProjectDispatcher::new());
    let (stop_a, agent_a) = start(config(address, "device-a"), dispatcher_a.clone());
    let (stop_b, agent_b) = start(
        config(address, "device-b"),
        Arc::new(LocalProjectDispatcher::new()),
    );
    wait_count(&broker, 2).await;
    read(
        &broker,
        "chat-a",
        "device-a",
        root.path(),
        "request-a-1234567890",
    )
    .await;
    read(
        &broker,
        "chat-b",
        "device-b",
        root.path(),
        "request-b-1234567890",
    )
    .await;
    let projects = dispatcher_a.restart_projects(chrono::Utc::now()).await;
    assert_eq!(projects.len(), 1);
    stop_a.send_replace(true);
    finish(agent_a).await;
    wait_count(&broker, 1).await;
    // Device B remains reachable while A is absent; no shared shutdown/fork is permitted.
    read(
        &broker,
        "chat-b",
        "device-b",
        root.path(),
        "request-b2-1234567890",
    )
    .await;
    assert!(!agent_b.is_finished());
    let replacement = Arc::new(LocalProjectDispatcher::new());
    replacement
        .restore_restart_projects(&projects)
        .await
        .unwrap();
    let restored = replacement.restart_projects(chrono::Utc::now()).await;
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].thread_id, projects[0].thread_id);
    assert_eq!(restored[0].root, projects[0].root);
    let (stop_replacement, agent_replacement) = start(config(address, "device-a"), replacement);
    wait_count(&broker, 2).await;
    read(
        &broker,
        "chat-a",
        "device-a",
        root.path(),
        "request-a2-1234567890",
    )
    .await;
    read(
        &broker,
        "chat-b",
        "device-b",
        root.path(),
        "request-b3-1234567890",
    )
    .await;
    stop_replacement.send_replace(true);
    stop_b.send_replace(true);
    finish(agent_replacement).await;
    finish(agent_b).await;
    wait_count(&broker, 0).await;
    drop(cleanup);
    finish(server).await;
}
