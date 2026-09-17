use std::{future::Future, path::Path, sync::Arc};

use cdr_remote_agent::{
    config::RemoteMcpConfig,
    restart_handoff::RestartHandoffRuntime,
    runner::{ManagedRemoteAgent, RemoteAgentControl},
    status::RemoteAgentStatus,
};
use tokio::sync::watch;
use tokio::time::{Duration, MissedTickBehavior, interval};

use crate::{
    runtime_paths::RuntimePaths,
    session_mirror_worker::{
        DiscordSessionMirrorSender, SessionMirrorWorker, run_session_mirror_worker,
    },
};

use super::{
    DiscordRuntimeError,
    worker_supervision::{MonitoredWorker, WorkerExitNotifier, spawn_monitored},
};
use crate::reserve_auto::ReserveAutoController;
use crate::{app_backend::AppServerTurnBackend, queue_runner::QueueCoordinator};

pub(super) fn spawn_unit_worker<F>(
    name: &'static str,
    notifier: WorkerExitNotifier,
    future: F,
) -> MonitoredWorker
where
    F: Future<Output = ()> + Send + 'static,
{
    spawn_monitored(name, notifier, async move {
        future.await;
        Ok(())
    })
}

pub(super) struct PreparedRemoteWorker {
    agent: ManagedRemoteAgent,
    control: RemoteAgentControl,
}

pub(super) async fn prepare_remote_worker(
    config: Option<RemoteMcpConfig>,
    status: RemoteAgentStatus,
    root: &Path,
) -> Result<Option<PreparedRemoteWorker>, DiscordRuntimeError> {
    let Some(remote) = config else {
        return Ok(None);
    };
    let handoff = RestartHandoffRuntime::system(root)?;
    let control = RemoteAgentControl::default();
    let agent = ManagedRemoteAgent::initialize(remote, status, handoff, control.clone()).await?;
    Ok(Some(PreparedRemoteWorker { agent, control }))
}

pub(super) fn start_remote_worker(
    prepared: Option<PreparedRemoteWorker>,
    shutdown: watch::Receiver<bool>,
    exit_notifier: WorkerExitNotifier,
) -> RemoteWorker {
    let Some(PreparedRemoteWorker { agent, control }) = prepared else {
        return (None, None);
    };
    let worker = spawn_monitored("remote-mcp", exit_notifier, async move {
        agent.run(shutdown).await.map_err(Into::into)
    });
    (Some(worker), Some(control))
}

type RemoteWorker = (Option<MonitoredWorker>, Option<RemoteAgentControl>);

pub(super) fn start_session_mirror_worker(
    enabled: bool,
    paths: &RuntimePaths,
    http: &Arc<twilight_http::Client>,
    shutdown: watch::Receiver<bool>,
    exit_notifier: WorkerExitNotifier,
) -> Option<MonitoredWorker> {
    enabled.then(|| {
        spawn_unit_worker(
            "session-mirror",
            exit_notifier,
            run_session_mirror_worker(
                SessionMirrorWorker::new(
                    paths.state_db.clone(),
                    paths.mirror_db.clone(),
                    Arc::new(DiscordSessionMirrorSender::new(
                        Arc::clone(http),
                        paths.mirror_db.clone(),
                    )),
                ),
                shutdown,
            ),
        )
    })
}

pub(super) fn start_reserve_auto_worker(
    controller: Option<Arc<ReserveAutoController>>,
    queue: Arc<QueueCoordinator<AppServerTurnBackend>>,
    mut shutdown: watch::Receiver<bool>,
    exit_notifier: WorkerExitNotifier,
) -> Option<MonitoredWorker> {
    controller.map(|controller| {
        spawn_unit_worker("reserve-auto", exit_notifier, async move {
            let mut ticker = interval(Duration::from_secs(30));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            let mut cursor: Option<String> = None;
            loop {
                if *shutdown.borrow() {
                    return;
                }
                tokio::select! {
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() { return; }
                    }
                    _ = ticker.tick() => {
                        if crate::reserve_auto::run_recovery_cycle(
                            &controller, &queue, &mut shutdown, &mut cursor,
                            Duration::from_secs(20),
                        ).await { return; }
                    }
                }
            }
        })
    })
}
