use super::ResidentAppServer;
use crate::{
    AppServerError,
    requests::{ThreadSettingsUpdate, update_thread_settings},
};

impl ResidentAppServer {
    pub async fn update_settings_with_watermark(
        &self,
        thread: &str,
        update: &ThreadSettingsUpdate,
        generation: u64,
    ) -> Result<u64, AppServerError> {
        let admission = self.state.admit_request(Some(generation))?;
        let request = update_thread_settings(thread, update);
        let mutation = self
            .prepare_mutation(request.method, &request.params, admission.generation)
            .await?;
        let super::target_mutation::Prepared::Ready(permit) = mutation else {
            return Err(crate::idle_release::held("unexpected settings admission"));
        };
        if let Some(fence) = &self.dead_generation_fence {
            fence.check_request(admission.generation, request.method, &request.params)?;
        }
        let mut watermark = None;
        let mut written = self.state.track_written_request(admission.generation);
        let result = admission
            .client
            .request_admitted_with_checks(
                request.method,
                request.params.clone(),
                request.timeout,
                || {
                    self.check_actual_mutation(
                        permit.as_ref(),
                        admission.generation,
                        request.method,
                        &request.params,
                    )
                },
                || {
                    // This hook runs with the transport writer acquired, immediately
                    // before write_all. Notifications observed while waiting are old.
                    watermark = Some(
                        admission
                            .client
                            .inner
                            .state
                            .lock()
                            .expect("runtime state lock")
                            .notification_revision(),
                    );
                    written.confirm_write_started();
                },
                || {},
            )
            .await;
        if matches!(result, Err(AppServerError::Timeout { .. })) {
            self.state.mark_timeout(admission.generation);
        }
        written.finish(&result);
        result?;
        Ok(watermark.expect("successful request was admitted by writer"))
    }
}
