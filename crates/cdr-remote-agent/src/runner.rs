use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::Utc;
use thiserror::Error;
use tokio::sync::watch;

use crate::bridge::connect_and_serve_until_with_status;
use crate::config::RemoteMcpConfig;
use crate::dispatcher::LocalProjectDispatcher;
use crate::restart_handoff::{RestartHandoffLifecycleError, RestartHandoffRuntime};
use crate::status::RemoteAgentStatus;

#[derive(Clone, Default)]
pub struct RemoteAgentControl {
    restart_handoff: Arc<AtomicBool>,
}

impl RemoteAgentControl {
    pub fn request_restart_handoff(&self) {
        self.restart_handoff.store(true, Ordering::Release);
    }

    fn restart_handoff_requested(&self) -> bool {
        self.restart_handoff.load(Ordering::Acquire)
    }
}

#[derive(Debug, Error)]
pub enum RemoteAgentRunError {
    #[error(transparent)]
    RestartHandoff(#[from] RestartHandoffLifecycleError),
}

pub struct ManagedRemoteAgent {
    config: RemoteMcpConfig,
    status: RemoteAgentStatus,
    handoff: RestartHandoffRuntime,
    control: RemoteAgentControl,
    dispatcher: Arc<LocalProjectDispatcher>,
}

impl ManagedRemoteAgent {
    pub async fn initialize(
        config: RemoteMcpConfig,
        status: RemoteAgentStatus,
        handoff: RestartHandoffRuntime,
        control: RemoteAgentControl,
    ) -> Result<Self, RemoteAgentRunError> {
        let dispatcher = Arc::new(LocalProjectDispatcher::new());
        let restored = handoff.restore(&dispatcher, &config, Utc::now()).await?;
        if restored > 0 {
            eprintln!("remote_mcp_restart_handoff_restored projects={restored}");
        }
        Ok(Self {
            config,
            status,
            handoff,
            control,
            dispatcher,
        })
    }

    pub async fn run(self, shutdown: watch::Receiver<bool>) -> Result<(), RemoteAgentRunError> {
        run_loop(
            &self.config,
            shutdown,
            &self.status,
            Arc::clone(&self.dispatcher),
        )
        .await;
        if self.control.restart_handoff_requested() {
            let now = Utc::now();
            let prepared = self
                .handoff
                .prepare(&self.dispatcher, &self.config, now)
                .await?;
            if prepared {
                let count = self.dispatcher.restart_projects(now).await.len();
                eprintln!("remote_mcp_restart_handoff_prepared projects={count}");
            }
        }
        Ok(())
    }
}

pub async fn run_remote_agent(config: RemoteMcpConfig, shutdown: watch::Receiver<bool>) {
    run_remote_agent_with_status(config, shutdown, RemoteAgentStatus::default()).await;
}

pub async fn run_remote_agent_with_status(
    config: RemoteMcpConfig,
    shutdown: watch::Receiver<bool>,
    status: RemoteAgentStatus,
) {
    let dispatcher = Arc::new(LocalProjectDispatcher::new());
    run_loop(&config, shutdown, &status, dispatcher).await;
}

pub async fn run_remote_agent_managed(
    config: RemoteMcpConfig,
    shutdown: watch::Receiver<bool>,
    status: RemoteAgentStatus,
    handoff: RestartHandoffRuntime,
    control: RemoteAgentControl,
) -> Result<(), RemoteAgentRunError> {
    ManagedRemoteAgent::initialize(config, status, handoff, control)
        .await?
        .run(shutdown)
        .await
}

async fn run_loop(
    config: &RemoteMcpConfig,
    mut shutdown: watch::Receiver<bool>,
    status: &RemoteAgentStatus,
    dispatcher: Arc<LocalProjectDispatcher>,
) {
    let mut generation = 0_u64;
    loop {
        if *shutdown.borrow() {
            status.set_disconnected();
            return;
        }
        generation = generation.saturating_add(1);
        let result = connect_and_serve_until_with_status(
            config,
            Arc::clone(&dispatcher),
            generation,
            shutdown.clone(),
            status,
        )
        .await;
        status.set_disconnected();
        if *shutdown.borrow() {
            return;
        }
        log_disconnect(generation, &result);
        if wait_reconnect(config.reconnect_delay_seconds, &mut shutdown).await {
            status.set_disconnected();
            return;
        }
    }
}

fn log_disconnect(generation: u64, result: &Result<(), crate::bridge::BridgeError>) {
    match result {
        Ok(()) => eprintln!("remote_mcp_bridge_disconnected generation={generation}"),
        Err(error) => {
            eprintln!("remote_mcp_bridge_disconnected generation={generation} error={error}");
        }
    }
}

async fn wait_reconnect(seconds: u64, shutdown: &mut watch::Receiver<bool>) -> bool {
    let delay = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(delay);
    tokio::select! {
        () = &mut delay => false,
        changed = shutdown.changed() => changed.is_err() || *shutdown.borrow(),
    }
}
