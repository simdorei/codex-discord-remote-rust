use std::future::Future;
use std::sync::Arc;

use serde_json::json;
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;

use super::{AppServerClient, AppServerConfig, Inner};
use crate::client::ClientLifecycle;
use crate::diagnostics::BoundedDiagnostics;
use crate::process::{SpawnedAppServer, spawn_app_server};
use crate::state::RuntimeState;
use crate::transport::{drain_stderr, drain_stdout};
use crate::{APP_SERVER_INITIALIZE_TIMEOUT, AppServerError};

impl AppServerClient {
    pub async fn start(config: AppServerConfig) -> Result<Self, AppServerError> {
        Self::start_observed(config, |_| ())
            .await
            .map(|(client, ())| client)
    }

    pub(crate) async fn start_observed<T>(
        config: AppServerConfig,
        observe: impl FnOnce(&Self) -> T,
    ) -> Result<(Self, T), AppServerError> {
        start_observed_using(
            config,
            |client| Ok(observe(client)),
            |_| {},
            |client| async move { client.close().await },
        )
        .await
    }

    pub(crate) async fn start_observed_fallible<T>(
        config: AppServerConfig,
        observe: impl FnOnce(&Self) -> Result<T, AppServerError>,
    ) -> Result<(Self, T), AppServerError> {
        start_observed_using(
            config,
            observe,
            |_| {},
            |client| async move { client.close().await },
        )
        .await
    }
}

async fn start_observed_using<T, F, Fut>(
    config: AppServerConfig,
    observe: impl FnOnce(&AppServerClient) -> Result<T, AppServerError>,
    after_initialized: impl FnOnce(&AppServerClient),
    cleanup: F,
) -> Result<(AppServerClient, T), AppServerError>
where
    F: FnOnce(AppServerClient) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), AppServerError>> + Send + 'static,
{
    let client = spawn_client(&config)?;
    let cleanup = StartupCleanup::spawn(client.clone(), cleanup);
    let observer = match observe(&client) {
        Ok(observer) => observer,
        Err(primary) => return Err(cleanup.fail(primary).await),
    };
    let initialize = json!({
        "clientInfo": {
            "name": config.client_name,
            "title": config.client_title,
            "version": config.client_version
        },
        "capabilities": {"experimentalApi": true}
    });
    if let Err(primary) = client
        .request("initialize", initialize, APP_SERVER_INITIALIZE_TIMEOUT)
        .await
    {
        return Err(cleanup.fail(primary).await);
    }
    if let Err(primary) = client.notify("initialized", json!({})).await {
        return Err(cleanup.fail(primary).await);
    }
    after_initialized(&client);
    let committed = client.inner.lifecycle.with_open(|| {
        let mut state = client.inner.state.lock().expect("runtime state lock");
        state.initialized = true;
        state.generation = 1;
    });
    if let Err(primary) = committed {
        return Err(cleanup.fail(primary).await);
    }
    cleanup.commit();
    Ok((client, observer))
}

fn spawn_client(config: &AppServerConfig) -> Result<AppServerClient, AppServerError> {
    let SpawnedAppServer {
        child,
        process_id,
        stdin,
        stdout,
        stderr,
    } = spawn_app_server(config)?;
    let (notifications, _) = broadcast::channel(1_000);
    let (server_requests, _) = broadcast::channel(500);
    let inner = Arc::new(Inner {
        child: tokio::sync::Mutex::new(Some(child)),
        closed: std::sync::atomic::AtomicBool::new(false),
        diagnostics: std::sync::Mutex::new(BoundedDiagnostics::default()),
        lifecycle: Arc::new(ClientLifecycle::new()),
        notifications,
        pending: std::sync::Mutex::new(std::collections::HashMap::new()),
        server_requests,
        state: std::sync::Mutex::new(RuntimeState::starting(process_id)),
        stdin: tokio::sync::Mutex::new(Some(stdin)),
        #[cfg(test)]
        write_pause: std::sync::Mutex::new(None),
    });
    tokio::spawn(drain_stdout(Arc::clone(&inner), stdout));
    tokio::spawn(drain_stderr(Arc::clone(&inner), stderr));
    Ok(AppServerClient { inner })
}

struct StartupCleanup {
    commit: Option<oneshot::Sender<()>>,
    task: JoinHandle<Result<(), AppServerError>>,
}

impl StartupCleanup {
    fn spawn<F, Fut>(client: AppServerClient, cleanup: F) -> Self
    where
        F: FnOnce(AppServerClient) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), AppServerError>> + Send + 'static,
    {
        let (commit, committed) = oneshot::channel();
        let task = tokio::spawn(async move {
            match committed.await {
                Ok(()) => Ok(()),
                Err(_) => cleanup(client).await,
            }
        });
        Self {
            commit: Some(commit),
            task,
        }
    }

    fn commit(mut self) {
        if let Some(commit) = self.commit.take() {
            let _ = commit.send(());
        }
    }

    async fn fail(mut self, primary: AppServerError) -> AppServerError {
        drop(self.commit.take());
        let cleanup = match self.task.await {
            Ok(result) => result,
            Err(error) => Err(AppServerError::StartupCleanupTask {
                message: error.to_string(),
            }),
        };
        match cleanup {
            Ok(()) => primary,
            Err(cleanup) => AppServerError::StartupCleanup {
                primary: Box::new(primary),
                cleanup: Box::new(cleanup),
            },
        }
    }
}

#[cfg(test)]
pub(super) async fn start_observed_with_cleanup<T, F, Fut>(
    config: AppServerConfig,
    observe: impl FnOnce(&AppServerClient) -> T,
    cleanup: F,
) -> Result<(AppServerClient, T), AppServerError>
where
    F: FnOnce(AppServerClient) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), AppServerError>> + Send + 'static,
{
    start_observed_using(config, |client| Ok(observe(client)), |_| {}, cleanup).await
}

#[cfg(test)]
pub(super) async fn start_observed_after_initialized<T>(
    config: AppServerConfig,
    observe: impl FnOnce(&AppServerClient) -> T,
    after_initialized: impl FnOnce(&AppServerClient),
) -> Result<(AppServerClient, T), AppServerError> {
    start_observed_using(
        config,
        |client| Ok(observe(client)),
        after_initialized,
        |client| async move { client.close().await },
    )
    .await
}
