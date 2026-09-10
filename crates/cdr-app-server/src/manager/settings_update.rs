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
        if let Some(fence) = &self.dead_generation_fence {
            fence.check_request(admission.generation, request.method, &request.params)?;
        }
        let mut watermark = None;
        let mut written = self.state.track_written_request(admission.generation);
        let result = admission
            .client
            .request_admitted_with_hook(request.method, request.params, request.timeout, || {
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
            })
            .await;
        if matches!(result, Err(AppServerError::Timeout { .. })) {
            self.state.mark_timeout(admission.generation);
        }
        written.finish(&result);
        result?;
        Ok(watermark.expect("successful request was admitted by writer"))
    }
}
