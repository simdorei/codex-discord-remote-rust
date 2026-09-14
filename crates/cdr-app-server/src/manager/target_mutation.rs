use super::ResidentAppServer;
use crate::idle_release::{
    IdleReleaseJournal,
    gate::{MutationAdmission, MutationPermit},
    held,
};
use crate::{
    AppServerClient, AppServerError, RequestId, ServerRequestOccurrence, extract_thread_id,
};
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) enum Prepared {
    Ready(Option<MutationPermit>),
    Completed(Value),
}

impl ResidentAppServer {
    pub fn install_idle_release_journal(
        &self,
        journal: Arc<dyn IdleReleaseJournal>,
    ) -> Result<(), AppServerError> {
        self.target_gate.install(journal)
    }

    /// A gap only disqualifies optional release, never ordinary execution.
    pub fn mark_idle_observation_gap(&self) {
        self.target_gate.mark_gap();
    }

    pub fn confirm_idle_observation(&self, generation: u64, notification: &crate::Notification) {
        if let Ok(admission) = self.state.admit_response(generation) {
            admission
                .client
                .inner
                .state
                .lock()
                .expect("runtime state lock")
                .confirm_idle_observation(notification);
        }
    }

    pub(super) fn settle_exited_idle_owner(
        &self,
        client: &AppServerClient,
        generation: u64,
    ) -> Result<(), AppServerError> {
        // The exact child slot is removed only after wait() proved exit. A closed
        // pipe, different generation or new instance is not equivalent evidence.
        if client
            .inner
            .state
            .lock()
            .expect("runtime state lock")
            .process_exit_confirmed
            && let Some(journal) = self.target_gate.journal()
        {
            journal.old_child_exited(self.instance_id(), generation)?;
        }
        Ok(())
    }

    pub fn subscription_resume_required(&self, thread: &str) -> Result<bool, AppServerError> {
        self.target_gate
            .journal()
            .map_or(Ok(false), |j| j.resume_required(thread))
    }

    pub(super) async fn prepare_mutation(
        &self,
        method: &str,
        params: &Value,
        generation: u64,
    ) -> Result<Prepared, AppServerError> {
        if matches!(
            method,
            "thread/read"
                | "thread/goal/get"
                | "thread/list"
                | "thread/loaded/list"
                | "model/list"
                | "account/rateLimits/read"
                | "account/usage/read"
                | "thread/start"
        ) {
            // thread/start creates a distinct new ID; it cannot mutate a held existing target.
            return Ok(Prepared::Ready(None));
        }
        if let Some(fence) = &self.dead_generation_fence {
            fence.check_request(generation, method, params)?;
        }
        let target = extract_thread_id(params);
        match self
            .target_gate
            .admit(self.instance_id(), generation, target.clone())?
        {
            MutationAdmission::Ordinary(permit) => Ok(Prepared::Ready(Some(permit))),
            MutationAdmission::Resubscribe(permit, token) => {
                let resume = if method == "thread/resume" {
                    params.clone()
                } else {
                    json!({"threadId":token.thread_id})
                };
                let result = self.resubscribe_managed(permit, token, resume).await?;
                if method == "thread/resume" {
                    return Ok(Prepared::Completed(result));
                }
                match self
                    .target_gate
                    .admit(self.instance_id(), generation, target)?
                {
                    MutationAdmission::Ordinary(permit) => Ok(Prepared::Ready(Some(permit))),
                    MutationAdmission::Resubscribe(_, _) => Err(held(
                        "unexpected second resubscription; target remains held",
                    )),
                }
            }
        }
    }

    pub(super) fn check_actual_mutation(
        &self,
        permit: Option<&MutationPermit>,
        generation: u64,
        method: &str,
        params: &Value,
    ) -> Result<(), AppServerError> {
        if self.generation() != generation {
            return Err(AppServerError::GenerationMismatch {
                expected: generation,
                actual: self.generation(),
            });
        }
        if let Some(permit) = permit {
            permit.preflight()?;
        }
        if let Some(fence) = &self.dead_generation_fence {
            fence.check_request(generation, method, params)?;
        }
        Ok(())
    }

    pub(super) async fn response_permit(
        &self,
        client: &AppServerClient,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
        generation: u64,
    ) -> Result<Option<MutationPermit>, AppServerError> {
        let params = {
            let state = client.inner.state.lock().expect("runtime state lock");
            state
                .server_response_candidate(id, occurrence)?
                .params
                .clone()
        };
        match self
            .prepare_mutation("server/response", &params, generation)
            .await?
        {
            Prepared::Ready(permit) => Ok(permit),
            Prepared::Completed(_) => Err(held("unexpected response admission")),
        }
    }
}
