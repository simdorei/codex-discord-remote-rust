use super::{CompletionWorker, CompletionWorkerError, i64_channel};
use cdr_store::queue::{QueueJobState, list};
use std::collections::BTreeSet;

#[cfg(test)]
#[path = "typing_revocation_tests.rs"]
mod revocation_tests;
#[cfg(test)]
#[path = "typing_http_tests.rs"]
mod tests;

impl CompletionWorker {
    pub(super) async fn send_typing(&self) -> Result<(), CompletionWorkerError> {
        let mut lifecycle = self.server.subscribe_lifecycle_changes();
        let generation = self.server.generation();
        let snapshot = self.server.lifecycle_snapshot().await;
        if !snapshot.healthy || snapshot.quarantined || snapshot.restart_pending {
            return Ok(());
        }
        let retention_version = self.terminal_fence.retention_version();
        let db = self.queue.db_path().to_owned();
        let (jobs, terminals) = tokio::task::spawn_blocking(move || {
            Ok::<_, cdr_store::StoreError>((
                list(&db)?,
                cdr_store::observed_completion::pending(&db)?,
            ))
        })
        .await
        .map_err(|error| {
            CompletionWorkerError::Delivery(format!("typing state lookup failed: {error}"))
        })??;
        self.terminal_fence
            .retain(generation, &jobs, retention_version);
        let mut channels = BTreeSet::new();
        let mut first_error = None;
        for job in jobs {
            let Some(turn) = job.turn_id.as_deref() else {
                continue;
            };
            if job.state != QueueJobState::Running
                || job.goal_waiting
                || channels.contains(&job.channel_id)
                || terminals
                    .iter()
                    .any(|(thread, id, _)| thread == &job.target_thread_id && id == turn)
            {
                continue;
            }
            let mut revoked = self.terminal_fence.subscribe();
            let active = match self.server.active_turn_id(&job.target_thread_id).await {
                Ok(active) => active,
                Err(error) => {
                    first_error.get_or_insert(CompletionWorkerError::AppServer(error));
                    continue;
                }
            };
            if active.as_deref() != Some(turn)
                || self.server.generation() != generation
                || lifecycle.has_changed().unwrap_or(true)
                || self
                    .terminal_fence
                    .stopped(generation, &job.target_thread_id, turn)
            {
                continue;
            }
            channels.insert(job.channel_id);
            let channel = match i64_channel(job.channel_id) {
                Ok(channel) => channel,
                Err(error) => {
                    first_error.get_or_insert(error);
                    continue;
                }
            };
            // Once admitted, an in-flight HTTP operation can race with terminal
            // observation; cancel it on revocation and admit no new request.
            tokio::select! {
                biased;
                _ = lifecycle.changed()=>return first_error.map_or(Ok(()), Err),
                _ = revoked.changed()=>{},
                result=self.http.create_typing_trigger(channel)=>{
                    if let Err(error)=result {
                        eprintln!("typing_channel_error channel_id={channel} error={error}");
                        first_error.get_or_insert(CompletionWorkerError::Delivery(error.to_string()));
                    }
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}
