use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{Mutex as AsyncMutex, broadcast};
use uuid::Uuid;

use crate::diagnostics::BoundedDiagnostics;
use crate::process::{AppServerInput, AppServerProcess};
use crate::rpc::{notification_value, request_value};
use crate::state::RuntimeState;
use crate::{AppServerError, Notification, RequestId, ServerRequest};

mod lifecycle;
mod pending;
mod startup;
#[cfg(test)]
#[path = "client/startup_close_race_tests.rs"]
mod startup_close_race_tests;
#[cfg(test)]
#[path = "client/startup_tests.rs"]
pub(crate) mod startup_tests;
#[cfg(test)]
mod write_test;

pub(crate) use lifecycle::{AdmissionPermit, ClientLifecycle};
#[cfg(test)]
pub(crate) use pending::insert as insert_pending_response;
pub(crate) use pending::{PendingOutcome, PendingResponse, take as take_pending_response};
#[cfg(test)]
pub(crate) use write_test::WriteTestPause;

pub const DEFAULT_CLIENT_NAME: &str = "codex-discord-remote";
pub const DEFAULT_CLIENT_TITLE: &str = "Codex Discord Remote";

#[derive(Debug, Clone)]
pub struct AppServerConfig {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub client_name: String,
    pub client_title: String,
    pub client_version: String,
}

impl AppServerConfig {
    #[must_use]
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            arguments: vec!["app-server".to_owned(), "--stdio".to_owned()],
            environment: BTreeMap::new(),
            client_name: DEFAULT_CLIENT_NAME.to_owned(),
            client_title: DEFAULT_CLIENT_TITLE.to_owned(),
            client_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }

    #[must_use]
    pub fn with_environment(
        mut self,
        environment: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        self.environment.extend(environment);
        self
    }
}

pub(crate) struct Inner {
    pub(crate) child: AsyncMutex<Option<AppServerProcess>>,
    pub(crate) closed: AtomicBool,
    pub(crate) diagnostics: Mutex<BoundedDiagnostics>,
    pub(crate) lifecycle: Arc<ClientLifecycle>,
    pub(crate) notifications: broadcast::Sender<Notification>,
    pub(crate) pending: Mutex<HashMap<RequestId, PendingResponse>>,
    pub(crate) server_requests: broadcast::Sender<ServerRequest>,
    pub(crate) state: Mutex<RuntimeState>,
    pub(crate) stdin: AsyncMutex<Option<AppServerInput>>,
    #[cfg(test)]
    pub(crate) write_pause: Mutex<Option<Arc<WriteTestPause>>>,
}

#[derive(Clone)]
pub struct AppServerClient {
    pub(crate) inner: Arc<Inner>,
}

impl AppServerClient {
    pub async fn request(
        &self,
        method: &str,
        params: Value,
        wait: Duration,
    ) -> Result<Value, AppServerError> {
        let _permit = self.admit_operation()?;
        self.request_admitted(method, params, wait).await
    }

    pub(crate) async fn request_admitted(
        &self,
        method: &str,
        params: Value,
        wait: Duration,
    ) -> Result<Value, AppServerError> {
        self.request_admitted_with_hook(method, params, wait, || {})
            .await
    }

    pub(crate) async fn request_admitted_with_hook(
        &self,
        method: &str,
        params: Value,
        wait: Duration,
        write_started: impl FnOnce(),
    ) -> Result<Value, AppServerError> {
        self.request_admitted_with_checks(method, params, wait, || Ok(()), write_started, || {})
            .await
    }

    pub(crate) async fn request_admitted_with_checks(
        &self,
        method: &str,
        params: Value,
        wait: Duration,
        preflight: impl FnOnce() -> Result<(), AppServerError>,
        write_started: impl FnOnce(),
        write_complete: impl FnOnce(),
    ) -> Result<Value, AppServerError> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(AppServerError::Closed);
        }
        let id = RequestId::String(Uuid::new_v4().to_string());
        let response_permit = self.admit_operation()?;
        let (pending, receiver) = PendingResponse::new(response_permit);
        let displaced = pending::insert(&self.inner, id.clone(), pending);
        drop(displaced);
        pending::spawn_deadline(Arc::downgrade(&self.inner), id.clone(), wait);
        let deadline = tokio::time::Instant::now() + wait;
        if let Err(error) = self
            .write_with_preflight(
                request_value(&id, method, &params),
                || {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(AppServerError::Timeout {
                            method: method.into(),
                            timeout_ms: wait.as_millis(),
                        });
                    }
                    preflight()
                },
                write_started,
            )
            .await
        {
            let removed = pending::take(&self.inner, &id);
            drop(removed);
            return Err(error);
        }
        write_complete();
        match receiver.await {
            Ok(PendingOutcome::Response(Ok(result))) => Ok(result),
            Ok(PendingOutcome::Response(Err(error))) => Err(AppServerError::Remote {
                method: method.to_owned(),
                code: error.code,
                message: error.message,
                data: error.data,
            }),
            Ok(PendingOutcome::TransportClosed { reason }) => {
                Err(AppServerError::TransportClosed {
                    method: method.to_owned(),
                    reason,
                })
            }
            Err(_) => Err(AppServerError::ResponseChannelClosed {
                method: method.to_owned(),
            }),
            Ok(PendingOutcome::Timeout) => Err(AppServerError::Timeout {
                method: method.to_owned(),
                timeout_ms: wait.as_millis(),
            }),
        }
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<(), AppServerError> {
        let _permit = self.admit_operation()?;
        self.write(notification_value(method, &params)).await
    }

    pub(crate) fn admit_operation(&self) -> Result<AdmissionPermit, AppServerError> {
        self.inner.lifecycle.admit()
    }
}
