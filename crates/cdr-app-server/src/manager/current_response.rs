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
        let permit = self
            .response_permit(&admission.client, id, occurrence, admission.generation)
            .await?;
        let attempt = super::target_mutation::ResponseAttempt::new(
            self,
            admission.generation,
            permit,
            crate::rpc::response_value(id, &result),
        );
        let mut written = self.state.track_written_request(admission.generation);
        let result = admission
            .client
            .respond_current_admitted_with_checks(
                id,
                occurrence,
                result,
                || attempt.begin(),
                || {
                    attempt.write_started();
                    written.confirm_write_started();
                },
            )
            .await;
        written.finish(&result);
        attempt.finish(result)
    }
}
