use super::{CompletionDeliveryIdentity, CompletionWorker, CompletionWorkerError, i64_channel};
use cdr_store::commentary_outbox::{self, PendingCommentary};

impl CompletionWorker {
    pub(super) fn ensure_delivery_order(
        &self,
        job: &str,
        before_commentary: Option<i64>,
    ) -> Result<(), CompletionWorkerError> {
        self.ensure_first_reply(job)?;
        if commentary_outbox::has_pending(self.queue.db_path(), job, before_commentary)? {
            return Err(CompletionWorkerError::Delivery(
                "output saved behind earlier undelivered progress; no output POST attempted".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn ensure_first_reply(&self, job: &str) -> Result<(), CompletionWorkerError> {
        if let Some(reason) = cdr_store::new_reply::output_hold(self.queue.db_path(), job)? {
            return Err(CompletionWorkerError::Held(reason));
        }
        if let Some(request) = cdr_store::first_reply::pending(self.queue.db_path(), job)? {
            return Err(CompletionWorkerError::Delivery(format!(
                "output saved; first reply is not confirmed for {request}. No output POST attempted; inspect the saved request if its reply failed"
            )));
        }
        Ok(())
    }

    pub(super) async fn deliver_commentary(
        &self,
        pending: &PendingCommentary,
    ) -> Result<(), CompletionWorkerError> {
        self.ensure_delivery_order(&pending.job_id, Some(pending.sequence))?;
        let identity = CompletionDeliveryIdentity::commentary(
            &pending.thread_id,
            &pending.turn_id,
            &pending.text,
        );
        self.send_idempotent_text(
            i64_channel(pending.channel_id)?,
            &identity,
            &super::delivery::commentary_message(&pending.text),
            &cdr_store::new_reply::DeliveryGuard {
                job_id: &pending.job_id,
                thread_id: &pending.thread_id,
                turn_id: &pending.turn_id,
            },
        )
        .await?;
        commentary_outbox::complete(self.queue.db_path(), pending.sequence)?;
        Ok(())
    }

    pub(super) async fn deliver_pending_commentary(&self) -> Result<(), CompletionWorkerError> {
        let mut first_error = None;
        for pending in commentary_outbox::pending(self.queue.db_path())? {
            if let Err(error) = self.deliver_commentary(&pending).await {
                eprintln!(
                    "commentary_pending_delivery_error job_id={} error={error}",
                    pending.job_id
                );
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}
