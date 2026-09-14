//! Managed maintenance futures are not aborted when their caller stops waiting.
use super::{
    ResidentAppServer,
    admission::{ResidentAdmission, ResidentState},
};
use crate::{
    AppServerError,
    idle_release::{ExclusivePermit, IdleReleaseToken, held},
};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

mod release;
mod transport;
#[cfg(test)]
mod transport_tests;

struct Work {
    state: ResidentState,
    admission: ResidentAdmission,
    permit: ExclusivePermit,
    token: IdleReleaseToken,
    fence: Option<Arc<dyn crate::DeadGenerationFence>>,
}

impl ResidentAppServer {
    fn idle_work(
        &self,
        permit: ExclusivePermit,
        token: IdleReleaseToken,
    ) -> Result<Work, AppServerError> {
        if token.owner_id != self.instance_id() {
            return Err(held("idle release owner mismatch"));
        }
        let admission = self.state.admit_request(Some(token.generation))?;
        Ok(Work {
            state: self.state.clone(),
            admission,
            permit,
            token,
            fence: self.dead_generation_fence.clone(),
        })
    }

    pub(super) async fn resubscribe_managed(
        &self,
        permit: ExclusivePermit,
        token: IdleReleaseToken,
        params: Value,
    ) -> Result<Value, AppServerError> {
        // Resubscribing was durably committed by admission, before this task exists.
        let work = self.idle_work(permit, token)?;
        tokio::spawn(async move { work.resubscribe(params).await })
            .await
            .map_err(|e| {
                held(format!(
                    "managed resubscription task failed; durable hold retained: {e}"
                ))
            })?
    }

    pub async fn release_idle_subscription(
        &self,
        token: IdleReleaseToken,
    ) -> Result<(), AppServerError> {
        if token.state != "Candidate" && token.state != "AwaitUnload" {
            return Err(held(
                "only Candidate or known-ACK AwaitUnload may run maintenance",
            ));
        }
        let permit = self.target_gate.reserve(&token)?;
        let work = self.idle_work(permit, token)?;
        tokio::spawn(async move { work.release().await })
            .await
            .map_err(|e| {
                held(format!(
                    "managed release task failed; durable hold retained: {e}"
                ))
            })?
    }
}

impl Work {
    fn advance(&mut self, state: &str, detail: &str) -> Result<(), AppServerError> {
        self.token = self.permit.journal.transition(&self.token, state, detail)?;
        Ok(())
    }

    async fn resubscribe(mut self, params: Value) -> Result<Value, AppServerError> {
        if self.token.state != "Resubscribing" {
            return Err(held("missing durable resume permission"));
        }
        let (result, phase) = self
            .rpc("thread/resume", params, Duration::from_secs(8), false, None)
            .await;
        match result {
            Ok(value)
                if value.pointer("/thread/id").and_then(Value::as_str)
                    == Some(self.token.thread_id.as_str())
                    && self.state.generation() == self.token.generation =>
            {
                // If this commit fails, Resubscribing remains a durable hold. No start.
                self.advance("Settled", "SupersededByConfirmedResubscribe")?;
                Ok(value)
            }
            other => {
                let error = other
                    .err()
                    .unwrap_or_else(|| held("resume returned wrong identity or owner changed"));
                let (state, detail) = if phase == transport::WritePhase::NotStarted {
                    ("AwaitUnload", "ResumeCancelledBeforeSend".to_owned())
                } else {
                    ("Unknown", error.to_string())
                };
                if let Err(recording) = self.advance(state, &detail) {
                    return Err(held(format!(
                        "{error}; recording failed: {recording}; durable resume hold retained"
                    )));
                }
                Err(error)
            }
        }
    }
}
