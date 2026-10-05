//! One bounded worker, never a detached task per stop command.
use super::{ActionError, ActionExecutor};
use crate::{queue_runner::TurnBackend, settings_binding::SettingsBinding};
use cdr_store::ingress::stop::control;
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;

impl<B: TurnBackend> ActionExecutor<B> {
    pub async fn run_stop_worker(self: Arc<Self>, mut shutdown: watch::Receiver<bool>) {
        let mut cursor = 0;
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while !*shutdown.borrow() {
            tokio::select! {
                biased;
                _=shutdown.changed()=>break,
                _=tick.tick()=>{}
            }
            let result = tokio::select! {
                biased;
                _=shutdown.changed()=>break,
                result=tokio::time::timeout(Duration::from_secs(2),self.process_stop_controls(&mut cursor))=>result,
            };
            match result {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => eprintln!("stop_control_worker: {error}"),
                Err(_) => eprintln!(
                    "stop_control_worker: cycle deadline; durable authority retained, no replay"
                ),
            }
        }
    }

    /// Keyset pagination advances even when an older target lock is busy.
    pub async fn process_stop_controls(&self, cursor: &mut i64) -> Result<usize, ActionError> {
        let server = self.server.as_ref().ok_or(ActionError::MissingAppServer)?;
        let pending = control::pending_after(&self.mirror_db, *cursor)?;
        if pending.is_empty() {
            *cursor = 0;
            return Ok(0);
        }
        let mut dispatched = 0;
        for (sequence, original) in pending {
            *cursor = sequence;
            if original.resident != server.instance_id()
                || u64::try_from(original.generation).ok() != Some(server.generation())
            {
                continue;
            }
            let Ok(_guard) = self.queue.target_lock(&original.target)?.try_lock_owned() else {
                continue;
            };
            let binding: SettingsBinding = serde_json::from_value(original.binding.clone())
                .map_err(|error| ActionError::Invalid(error.to_string()))?;
            let resolver = self.settings_resolver();
            resolver.validate_selected_snapshot(&binding)?;
            if self
                .verified_owned_turn(&original.target, Some(&original.turn))
                .await
                .is_err()
            {
                continue;
            }
            let Some(claim) = control::claim(&self.mirror_db, &original, || {
                resolver
                    .validate_selected_snapshot(&binding)
                    .map_err(|error| cdr_store::StoreError::Integrity(error.to_string()))
            })?
            else {
                continue;
            };
            let generation =
                u64::try_from(original.generation).map_err(|_| ActionError::IntegerRange)?;
            let mut request =
                cdr_app_server::requests::interrupt_turn(&original.target, &original.turn);
            request.timeout = Duration::from_secs(2);
            let result = server
                .execute_stop_control(
                    request,
                    generation,
                    serde_json::to_value(&claim)
                        .map_err(|error| ActionError::Invalid(error.to_string()))?,
                    Arc::new(move || {
                        resolver
                            .validate_selected_snapshot(&binding)
                            .map_err(|error| cdr_app_server::AppServerError::MutationHeld {
                                message: error.to_string(),
                            })
                    }),
                )
                .await;
            dispatched += 1;
            if let Err(error) = result {
                control::record_error(&self.mirror_db, &claim, &error.to_string())?;
                eprintln!(
                    "stop_control_unknown operation={} error={error}",
                    original.operation_id
                );
            }
        }
        Ok(dispatched)
    }
}
