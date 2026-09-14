use crate::{AppServerClient, AppServerError, RequestId, ServerRequestOccurrence};
use serde_json::Value;

impl AppServerClient {
    pub async fn respond_current(
        &self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
        result: Value,
    ) -> Result<(), AppServerError> {
        let _permit = self.admit_operation()?;
        self.respond_current_admitted_with_hook(id, occurrence, result, || {})
            .await
    }

    pub(crate) async fn respond_current_admitted_with_hook(
        &self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
        result: Value,
        write_started: impl FnOnce(),
    ) -> Result<(), AppServerError> {
        self.respond_current_admitted_with_checks(id, occurrence, result, || Ok(()), write_started)
            .await
    }

    pub(crate) async fn respond_current_admitted_with_checks(
        &self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
        result: Value,
        preflight: impl FnOnce() -> Result<(), AppServerError>,
        write_started: impl FnOnce(),
    ) -> Result<(), AppServerError> {
        let claim = self
            .write_with_preflight(
                crate::rpc::response_value(id, &result),
                || {
                    preflight()?;
                    super::response_claim::ServerResponseClaim::begin_current(self, id, occurrence)
                },
                write_started,
            )
            .await?;
        #[cfg(test)]
        self.pause_before_response_resolve().await;
        claim.resolve()
    }
}
