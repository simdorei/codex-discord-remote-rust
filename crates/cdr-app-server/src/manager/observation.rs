use super::ResidentAppServer;
use crate::{AppServerError, observation::ObservationWindow};

impl ResidentAppServer {
    #[must_use]
    pub fn observation_tracking_enabled(&self) -> bool {
        self.target_gate
            .journal()
            .is_some_and(|j| j.tracks_observations())
    }
    pub fn observation_window(
        &self,
        generation: u64,
        after: u64,
        upper: Option<u64>,
    ) -> Result<ObservationWindow, AppServerError> {
        let admission = self.state.admit_response(generation)?;
        let mut page = admission
            .client
            .inner
            .state
            .lock()
            .expect("runtime state lock")
            .observation_window(after, upper)?;
        self.instance_id().clone_into(&mut page.owner_id);
        page.generation = generation;
        Ok(page)
    }
    /// Mark a source-stream loss, not an exact number of missing source packets.
    pub fn mark_source_observation_gap(&self, generation: u64) {
        self.target_gate.mark_gap();
        let result = (|| {
            let journal = self
                .target_gate
                .journal()
                .ok_or_else(|| crate::idle_release::held("observation journal absent"))?;
            let upper = self
                .observation_window(generation, 0, Some(0))?
                .source_upper;
            journal.observe_source_upper(self.instance_id(), generation, upper)
        })();
        if let Err(error) = result {
            self.mark_idle_observation_gap();
            eprintln!("observation_gap_store_error error={error}");
        }
    }
    pub fn reconcile_idle_observation_prefix(
        &self,
        generation: u64,
        through: u64,
    ) -> Result<bool, AppServerError> {
        let epoch = self.target_gate.gap_epoch();
        let Some(journal) = self.target_gate.journal() else {
            return Ok(false);
        };
        if !journal.observation_scope_verified(self.instance_id(), generation, through)? {
            return Ok(false);
        }
        let admission = self.state.admit_response(generation)?;
        let accepted = admission
            .client
            .inner
            .state
            .lock()
            .expect("runtime state lock")
            .certify_observation_prefix(through);
        Ok(accepted && self.target_gate.clear_gap(epoch))
    }
}
