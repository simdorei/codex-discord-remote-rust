use super::{CompletionDeliveryIdentity, CompletionWorker, CompletionWorkerError, i64_channel};
use cdr_store::goal_progress::{self, PendingProgress};

impl CompletionWorker {
    pub(super) async fn deliver_goal_progress(
        &self,
        pending: &PendingProgress,
    ) -> Result<(), CompletionWorkerError> {
        let job = pending.job_id.as_deref().ok_or_else(|| CompletionWorkerError::Delivery(
            "legacy goal progress has no durable request identity; saved for review without sending".into(),
        ))?;
        self.ensure_first_reply(job)?;
        let identity = CompletionDeliveryIdentity::goal_progress(&pending.thread, &pending.turn);
        let result = self
            .send_idempotent_text(
                i64_channel(pending.channel)?,
                &identity,
                &pending.content,
                &cdr_store::new_reply::DeliveryGuard {
                    job_id: job,
                    thread_id: &pending.thread,
                    turn_id: &pending.turn,
                },
            )
            .await;
        if let Err(error) = result {
            if let Err(record_error) =
                goal_progress::record_error(self.queue.db_path(), pending, &error.to_string())
            {
                eprintln!("goal_progress_delivery_error: {error}; record_error: {record_error}");
                return Err(record_error.into());
            }
            return Err(error);
        }
        goal_progress::complete(self.queue.db_path(), pending)?;
        Ok(())
    }

    pub(super) async fn recover_goal_progress(&self) -> Result<(), CompletionWorkerError> {
        let mut first = None;
        for pending in goal_progress::pending(self.queue.db_path())? {
            if let Err(error) = self.deliver_goal_progress(&pending).await {
                eprintln!(
                    "goal_progress_pending thread={} turn={}: {error}",
                    pending.thread, pending.turn
                );
                if first.is_none() {
                    first = Some(error);
                }
            }
        }
        first.map_or(Ok(()), Err)
    }
}
