use super::{AppServerError, Arc, Duration, Value, Work, held};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum WritePhase {
    NotStarted,
    Partial,
    Flushed,
}

impl Work {
    pub(super) async fn rpc(
        &self,
        method: &str,
        params: Value,
        wait: Duration,
        require_idle: bool,
        watermark: Option<u64>,
    ) -> (Result<Value, AppServerError>, WritePhase) {
        let phase = AtomicU8::new(0);
        let mut written = self.state.track_written_request(self.token.generation);
        let result = self.admission.client.request_admitted_with_checks(method, params.clone(), wait,
            || {
                let snapshot = self.state.snapshot();
                if snapshot.generation != self.token.generation || !snapshot.accepting || snapshot.quarantined
                    || snapshot.client.as_ref().is_none_or(|c| !Arc::ptr_eq(&c.inner, &self.admission.client.inner)) {
                    return Err(held("maintenance connection changed or became unhealthy before actual write"));
                }
                self.permit.journal.verify(&self.token, require_idle)?;
                if let Some(fence) = &self.fence { fence.check_request(self.token.generation, method, &params)?; }
                if require_idle { self.local_idle(watermark)?; }
                Ok(())
            }, || { phase.store(1, Ordering::Release); written.confirm_write_started(); },
            || { phase.store(2, Ordering::Release); }).await;
        // The ordinary written guard still quarantines actual transport damage.
        // A healthy, fully flushed maintenance timeout does not mark_timeout().
        written.finish(&result);
        let phase = match phase.load(Ordering::Acquire) {
            0 => WritePhase::NotStarted,
            1 => WritePhase::Partial,
            _ => WritePhase::Flushed,
        };
        (result, phase)
    }

    pub(super) fn local_idle(&self, watermark: Option<u64>) -> Result<u64, AppServerError> {
        if !self.permit.gate.observations_verified() {
            return Err(held("idle unverified: observer gap or journal failure"));
        }
        let state = self
            .admission
            .client
            .inner
            .state
            .lock()
            .expect("runtime state lock");
        if !state.idle_observations_caught_up() {
            return Err(held("idle unverified: notifications not yet journaled"));
        }
        if state.active_turn_id(&self.token.thread_id).is_some()
            || state.unsettled_server_requests(None).iter().any(|r| {
                crate::extract_thread_id(&r.params).is_none_or(|t| t == self.token.thread_id)
            })
        {
            return Err(held(
                "idle unverified: active turn or unsettled server request",
            ));
        }
        let revision = state.notification_revision();
        if watermark.is_some_and(|expected| revision != expected) {
            return Err(held("idle observation changed before actual send"));
        }
        Ok(revision)
    }
}
