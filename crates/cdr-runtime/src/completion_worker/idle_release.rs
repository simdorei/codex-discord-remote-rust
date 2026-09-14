use super::{Arc, CompletionWorker, CompletionWorkerError, Duration, watch};

pub(super) async fn run(worker: Arc<CompletionWorker>, mut shutdown: watch::Receiver<bool>) {
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = shutdown.changed() => { if changed.is_err() || *shutdown.borrow() { return; } },
            _ = tick.tick() => {
                if *shutdown.borrow() { return; }
                if let Err(error) = worker.maintain_idle_subscriptions().await {
                    eprintln!("idle_release_maintenance_error error={error}");
                }
            }
        }
    }
}

impl CompletionWorker {
    pub(super) async fn maintain_idle_subscriptions(&self) -> Result<(), CompletionWorkerError> {
        for intent in cdr_store::idle_release::pending(self.queue.db_path())? {
            if intent.owner_id != self.server.instance_id()
                || u64::try_from(intent.generation).ok() != Some(self.server.generation())
                || !matches!(intent.state.as_str(), "Candidate" | "AwaitUnload")
            {
                continue;
            }
            let lock = self.queue.target_lock(&intent.thread_id)?;
            // Busy targets are revisited, never wait behind a user's active operation.
            let Ok(_guard) = lock.try_lock() else {
                continue;
            };
            let token = crate::idle_release::token(intent)?;
            if let Err(error) = self.server.release_idle_subscription(token).await {
                // Journal contains exact unresolved effect; no replay or global restart.
                eprintln!("idle_release_target_error error={error}");
            }
        }
        Ok(())
    }
}
