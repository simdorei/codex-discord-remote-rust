use super::ResidentAppServer;
use crate::{AppServerError, RequestId, ServerRequestOccurrence};
use serde_json::Value;

impl ResidentAppServer {
    /// Reply only while this exact occurrence's original turn is active at wire admission.
    pub async fn respond_current(
        &self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
        result: Value,
        expected_generation: u64,
    ) -> Result<(), AppServerError> {
        let admission = self.state.admit_response(expected_generation)?;
        let mut written = self.state.track_written_request(admission.generation);
        let result = admission
            .client
            .respond_current_admitted_with_hook(id, occurrence, result, || {
                written.confirm_write_started();
            })
            .await;
        written.finish(&result);
        result
    }
}
