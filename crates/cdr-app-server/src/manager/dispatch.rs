use super::{ResidentAppServer, target_mutation};
use crate::{
    AppServerError, RequestId, RpcErrorPayload, ServerRequestOccurrence, requests::AppRequest,
};
use serde_json::Value;
use std::time::Duration;

impl ResidentAppServer {
    pub async fn request(
        &self,
        method: &str,
        params: Value,
        wait: Duration,
        expected_generation: Option<u64>,
    ) -> Result<Value, AppServerError> {
        let admission = self.state.admit_request(expected_generation)?;
        let mutation = self
            .prepare_mutation(method, &params, admission.generation)
            .await?;
        let permit = match mutation {
            target_mutation::Prepared::Completed(value) => return Ok(value),
            target_mutation::Prepared::Ready(permit) => permit,
        };
        if let Some(fence) = &self.dead_generation_fence {
            fence.check_request(admission.generation, method, &params)?;
        }
        let mut written = self.state.track_written_request(admission.generation);
        let result = admission
            .client
            .request_admitted_with_checks(
                method,
                params.clone(),
                wait,
                || {
                    self.check_actual_mutation(
                        permit.as_ref(),
                        admission.generation,
                        method,
                        &params,
                    )
                },
                || written.confirm_write_started(),
                || {},
            )
            .await;
        if matches!(result, Err(AppServerError::Timeout { .. })) && method != "thread/read" {
            self.state.mark_timeout(admission.generation);
        }
        written.finish(&result);
        result
    }

    pub async fn execute(
        &self,
        request: AppRequest,
        expected_generation: Option<u64>,
    ) -> Result<Value, AppServerError> {
        self.request(
            request.method,
            request.params,
            request.timeout,
            expected_generation,
        )
        .await
    }

    pub async fn respond(
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
        let mut written = self.state.track_written_request(admission.generation);
        let result = admission
            .client
            .respond_admitted_with_checks(
                id,
                occurrence,
                result,
                || {
                    self.check_actual_mutation(
                        permit.as_ref(),
                        admission.generation,
                        "server/response",
                        &Value::Null,
                    )
                },
                || {
                    written.confirm_write_started();
                },
            )
            .await;
        written.finish(&result);
        result
    }

    pub async fn respond_error(
        &self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
        error: RpcErrorPayload,
        expected_generation: u64,
    ) -> Result<(), AppServerError> {
        let admission = self.state.admit_response(expected_generation)?;
        let permit = self
            .response_permit(&admission.client, id, occurrence, admission.generation)
            .await?;
        let mut written = self.state.track_written_request(admission.generation);
        let result = admission
            .client
            .respond_error_admitted_with_checks(
                id,
                occurrence,
                error,
                || {
                    self.check_actual_mutation(
                        permit.as_ref(),
                        admission.generation,
                        "server/response",
                        &Value::Null,
                    )
                },
                || {
                    written.confirm_write_started();
                },
            )
            .await;
        written.finish(&result);
        result
    }
}
