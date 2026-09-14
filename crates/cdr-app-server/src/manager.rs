use std::sync::{Arc, Mutex as StdMutex};

use serde_json::Value;
use tokio::sync::{Mutex as AsyncMutex, broadcast, watch};

use crate::{AppServerClient, AppServerConfig, AppServerError, ServerRequest};

mod admission;
mod close;
#[cfg(test)]
#[path = "manager/close_retry_tests.rs"]
mod close_retry_tests;
mod current_response;
mod dead_generation;
#[cfg(test)]
mod dead_generation_fence_tests;
mod death;
#[cfg(test)]
#[path = "manager/death_tests.rs"]
mod death_tests;
mod dispatch;
mod events;
mod idle_maintenance;
mod lifecycle_changes;
#[cfg(test)]
#[path = "manager/replacement_failure_tests.rs"]
mod replacement_failure_tests;
mod restart;
#[cfg(test)]
#[path = "manager/restart_close_error_tests.rs"]
mod restart_close_error_tests;
mod settings_update;
#[cfg(test)]
#[path = "manager/startup_cancel_tests.rs"]
mod startup_cancel_tests;
mod supervisor;
#[cfg(test)]
#[path = "manager/supervisor_backoff_tests.rs"]
mod supervisor_backoff_tests;
#[cfg(test)]
#[path = "manager/supervisor_lifecycle_tests.rs"]
mod supervisor_lifecycle_tests;
#[cfg(test)]
#[path = "manager/supervisor_safety_tests.rs"]
mod supervisor_safety_tests;
#[cfg(test)]
#[path = "manager/supervisor_state_race_tests.rs"]
mod supervisor_state_race_tests;
#[cfg(test)]
#[path = "manager/supervisor_tests.rs"]
mod supervisor_tests;
mod target_mutation;
#[cfg(test)]
#[path = "manager/write_cancel_tests.rs"]
mod write_cancel_tests;

use admission::ResidentState;
use events::{
    RESIDENT_NOTIFICATION_CAPACITY, RESIDENT_REQUEST_CAPACITY, ResidentForwarders,
    prepare_forwarders,
};
pub use events::{ResidentNotificationEvent, ResidentServerRequestEvent};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResidentLifecycleSnapshot {
    pub generation: u64,
    pub healthy: bool,
    pub quarantined: bool,
    pub restart_pending: bool,
    pub process_id: Option<u32>,
}

pub struct ResidentAppServer {
    instance_id: String,
    state: ResidentState,
    config: AppServerConfig,
    restart_lock: AsyncMutex<()>,
    notifications: broadcast::Sender<ResidentNotificationEvent>,
    server_requests: broadcast::Sender<ResidentServerRequestEvent>,
    forwarder_generation: watch::Sender<u64>,
    forwarders: StdMutex<Option<ResidentForwarders>>,
    dead_generation_fence: Option<Arc<dyn crate::DeadGenerationFence>>,
    target_gate: Arc<crate::idle_release::gate::TargetGate>,
}

impl ResidentAppServer {
    pub async fn start(config: AppServerConfig) -> Result<Self, AppServerError> {
        Self::start_using(config, None).await
    }

    pub async fn start_with_dead_generation_fence(
        config: AppServerConfig,
        fence: Arc<dyn crate::DeadGenerationFence>,
    ) -> Result<Self, AppServerError> {
        Self::start_using(config, Some(fence)).await
    }

    async fn start_using(
        config: AppServerConfig,
        dead_generation_fence: Option<Arc<dyn crate::DeadGenerationFence>>,
    ) -> Result<Self, AppServerError> {
        let (notifications, _) = broadcast::channel(RESIDENT_NOTIFICATION_CAPACITY);
        let (server_requests, _) = broadcast::channel(RESIDENT_REQUEST_CAPACITY);
        let (forwarder_generation, generation_rx) = watch::channel(1);
        let (client, forwarders) = AppServerClient::start_observed(config.clone(), |client| {
            prepare_forwarders(
                client,
                1,
                notifications.clone(),
                server_requests.clone(),
                generation_rx,
            )
        })
        .await?;
        let state = ResidentState::new(client.clone());
        let forwarders = forwarders.with_death_monitor(&client, state.clone(), 1);
        forwarders.activate();
        Ok(Self {
            instance_id: uuid::Uuid::new_v4().to_string(),
            state,
            config,
            restart_lock: AsyncMutex::new(()),
            notifications,
            server_requests,
            forwarder_generation,
            forwarders: StdMutex::new(Some(forwarders)),
            dead_generation_fence,
            target_gate: Arc::default(),
        })
    }

    pub async fn close(&self) -> Result<(), AppServerError> {
        close::close(self).await
    }

    #[must_use]
    pub fn subscribe_notifications(&self) -> broadcast::Receiver<ResidentNotificationEvent> {
        self.notifications.subscribe()
    }

    #[must_use]
    pub fn subscribe_server_requests(&self) -> broadcast::Receiver<ResidentServerRequestEvent> {
        self.server_requests.subscribe()
    }

    #[allow(clippy::unused_async)] // Preserve the existing async resident API.
    pub async fn lifecycle_snapshot(&self) -> ResidentLifecycleSnapshot {
        let state = self.state.snapshot();
        let child = state
            .client
            .as_ref()
            .map(AppServerClient::lifecycle_snapshot);
        ResidentLifecycleSnapshot {
            generation: state.generation,
            healthy: state.accepting
                && child.as_ref().is_some_and(|snapshot| snapshot.healthy)
                && !state.quarantined,
            quarantined: state.quarantined,
            restart_pending: state.restart_pending,
            process_id: child.and_then(|snapshot| snapshot.process_id),
        }
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.state.generation()
    }

    /// Generation numbers restart at one in a new resident. Never confuse two owners.
    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn observed_thread_settings(
        &self,
        thread_id: &str,
        expected_generation: u64,
    ) -> Result<Option<(u64, Value)>, AppServerError> {
        let actual = self.generation();
        if actual != expected_generation {
            return Err(AppServerError::GenerationMismatch {
                expected: expected_generation,
                actual,
            });
        }
        let result = self
            .state
            .current_client()?
            .observed_thread_settings(thread_id);
        let actual = self.generation();
        if actual != expected_generation {
            return Err(AppServerError::GenerationMismatch {
                expected: expected_generation,
                actual,
            });
        }
        Ok(result)
    }

    #[allow(clippy::unused_async)] // Preserve the existing async resident API.
    pub async fn active_turn_id(&self, thread_id: &str) -> Result<Option<String>, AppServerError> {
        Ok(self.state.current_client()?.active_turn_id(thread_id))
    }

    #[allow(clippy::unused_async)] // Preserve the existing async resident API.
    pub async fn pending_server_requests(
        &self,
        thread_id: Option<&str>,
    ) -> Result<Vec<ServerRequest>, AppServerError> {
        Ok(self
            .state
            .current_client()?
            .pending_server_requests(thread_id))
    }

    #[allow(clippy::unused_async)] // Preserve the existing async resident API.
    pub async fn has_unsettled_server_requests(&self) -> Result<bool, AppServerError> {
        Ok(self.state.current_client()?.has_unsettled_server_requests())
    }

    async fn stop_forwarders(&self) {
        let _ = self.forwarder_generation.send(0);
        let forwarders = self
            .forwarders
            .lock()
            .expect("resident forwarders lock")
            .take();
        if let Some(forwarders) = forwarders {
            forwarders.join().await;
        }
    }
}
