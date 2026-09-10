use serde_json::Value;
use tokio::sync::broadcast;

use crate::client::AppServerClient;
use crate::rpc::{error_value, response_value};
use crate::{
    AppServerError, DiagnosticSnapshot, LifecycleSnapshot, Notification, RequestId,
    RpcErrorPayload, ServerRequest, ServerRequestOccurrence,
};

mod close;
mod current_response;
mod response_claim;
mod write;

#[cfg(test)]
#[path = "control/request_occurrence_tests.rs"]
mod request_occurrence_tests;

#[cfg(test)]
#[path = "control/close_cleanup_tests.rs"]
mod close_cleanup_tests;

use response_claim::ServerResponseClaim;

impl AppServerClient {
    pub async fn respond(
        &self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
        result: Value,
    ) -> Result<(), AppServerError> {
        let _permit = self.admit_operation()?;
        self.respond_admitted(id, occurrence, result).await
    }

    pub(crate) async fn respond_admitted(
        &self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
        result: Value,
    ) -> Result<(), AppServerError> {
        self.respond_admitted_with_hook(id, occurrence, result, || {})
            .await
    }

    pub(crate) async fn respond_admitted_with_hook(
        &self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
        result: Value,
        write_started: impl FnOnce(),
    ) -> Result<(), AppServerError> {
        let claim = ServerResponseClaim::begin(self, id, occurrence)?;
        self.write_with_hook(response_value(id, &result), write_started)
            .await?;
        #[cfg(test)]
        self.pause_before_response_resolve().await;
        claim.resolve()
    }

    pub async fn respond_error(
        &self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
        error: RpcErrorPayload,
    ) -> Result<(), AppServerError> {
        let _permit = self.admit_operation()?;
        self.respond_error_admitted(id, occurrence, error).await
    }

    pub(crate) async fn respond_error_admitted(
        &self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
        error: RpcErrorPayload,
    ) -> Result<(), AppServerError> {
        self.respond_error_admitted_with_hook(id, occurrence, error, || {})
            .await
    }

    pub(crate) async fn respond_error_admitted_with_hook(
        &self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
        error: RpcErrorPayload,
        write_started: impl FnOnce(),
    ) -> Result<(), AppServerError> {
        let claim = ServerResponseClaim::begin(self, id, occurrence)?;
        self.write_with_hook(error_value(id, &error), write_started)
            .await?;
        #[cfg(test)]
        self.pause_before_response_resolve().await;
        claim.resolve()
    }

    #[cfg(test)]
    async fn pause_before_response_resolve(&self) {
        let pause = self
            .inner
            .write_pause
            .lock()
            .expect("write pause lock")
            .clone();
        if let Some(pause) = pause {
            pause.before_response_resolve().await;
        }
    }

    #[must_use]
    pub fn subscribe_notifications(&self) -> broadcast::Receiver<Notification> {
        self.inner.notifications.subscribe()
    }

    #[must_use]
    pub fn observed_thread_settings(&self, thread_id: &str) -> Option<(u64, Value)> {
        self.inner
            .state
            .lock()
            .expect("runtime state lock")
            .observed_thread_settings(thread_id)
    }

    #[must_use]
    pub fn subscribe_server_requests(&self) -> broadcast::Receiver<ServerRequest> {
        self.inner.server_requests.subscribe()
    }

    #[must_use]
    pub fn lifecycle_snapshot(&self) -> LifecycleSnapshot {
        self.inner
            .state
            .lock()
            .expect("runtime state lock")
            .snapshot()
    }

    #[must_use]
    pub fn diagnostic_snapshot(&self) -> DiagnosticSnapshot {
        self.inner
            .diagnostics
            .lock()
            .expect("diagnostic lock")
            .snapshot()
    }

    pub async fn wait_closed(&self) -> String {
        self.inner.lifecycle.wait_closed().await
    }

    #[must_use]
    pub fn pending_server_requests(&self, thread_id: Option<&str>) -> Vec<ServerRequest> {
        self.inner
            .state
            .lock()
            .expect("runtime state lock")
            .pending_server_requests(thread_id)
    }

    #[must_use]
    pub fn has_unsettled_server_requests(&self) -> bool {
        self.inner
            .state
            .lock()
            .expect("runtime state lock")
            .has_unsettled_server_requests()
    }

    #[must_use]
    pub fn latest_approval_request(&self, thread_id: &str) -> Option<ServerRequest> {
        self.inner
            .state
            .lock()
            .expect("runtime state lock")
            .latest_approval_request(thread_id)
    }

    #[must_use]
    pub fn latest_input_request(&self, thread_id: &str) -> Option<ServerRequest> {
        self.inner
            .state
            .lock()
            .expect("runtime state lock")
            .latest_input_request(thread_id)
    }

    #[must_use]
    pub fn active_turn_id(&self, thread_id: &str) -> Option<String> {
        self.inner
            .state
            .lock()
            .expect("runtime state lock")
            .active_turn_id(thread_id)
    }

    #[must_use]
    pub fn has_active_turns(&self) -> bool {
        self.inner
            .state
            .lock()
            .expect("runtime state lock")
            .has_active_turns()
    }

    pub(crate) fn seal_if_quiescent(&self) -> bool {
        self.inner.lifecycle.seal_if_quiescent(|| {
            let state = self.inner.state.lock().expect("runtime state lock");
            !state.has_active_turns() && !state.has_unsettled_server_requests()
        })
    }

    pub(crate) fn seal_admissions(&self) {
        self.inner.lifecycle.seal();
    }

    pub async fn cancel_pending_server_requests(
        &self,
        thread_id: &str,
    ) -> Result<usize, AppServerError> {
        let requests = self
            .inner
            .state
            .lock()
            .expect("runtime state lock")
            .unsettled_server_requests(Some(thread_id));
        let mut first_error = None;
        for request in &requests {
            let result = self
                .respond_error(
                    &request.id,
                    request.occurrence,
                    RpcErrorPayload {
                        code: -32_800,
                        message: "Request cancelled because the remote operation ended.".to_owned(),
                        data: None,
                    },
                )
                .await;
            if let Err(error) = result
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(requests.len())
    }

    pub async fn close(&self) -> Result<(), AppServerError> {
        close::close(&self.inner).await
    }
}
