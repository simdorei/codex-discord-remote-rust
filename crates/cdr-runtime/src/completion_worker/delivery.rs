use std::time::{SystemTime, UNIX_EPOCH};

use cdr_app_server::goal::ThreadGoalStatus;
use cdr_app_server::outcomes::{TurnCompletion, TurnStatus};
use cdr_discord::delivery::DeliveryPolicy;
use cdr_store::StoreError;
use cdr_store::delivery::{StoredDelivery, complete, list_pending, record_failure};
use twilight_model::id::{Id, marker::ChannelMarker};

use super::delivery_identity::{CompletionDeliveryIdentity, deliver_idempotent_chunks};
use super::{CompletionWorker, CompletionWorkerError};

const NO_VISIBLE_REPLY: &str = "Completed (no visible reply)";

impl CompletionWorker {
    pub(super) async fn deliver_pending(&self) -> Result<(), CompletionWorkerError> {
        let commentary = self.deliver_pending_commentary().await;
        let finals = attempt_all(list_pending(self.queue.db_path())?, |pending| async move {
            let delivery_id = pending.delivery_id.clone();
            let result = self.deliver_one(&pending).await;
            if let Err(error) = &result {
                eprintln!(
                    "completion_pending_delivery_error delivery_id={delivery_id} error={error}"
                );
            }
            result
        })
        .await;
        commentary.and(finals)
    }

    pub(super) async fn deliver_one(
        &self,
        pending: &StoredDelivery,
    ) -> Result<(), CompletionWorkerError> {
        let result = async {
            self.ensure_delivery_order(&pending.job_id, None)?;
            if cdr_store::goal_progress::has_pending_job(
                self.queue.db_path(),
                &pending.job_id,
                &pending.target_thread_id,
            )? {
                return Err(CompletionWorkerError::Delivery(
                    "final saved behind undelivered goal progress; no final POST attempted".into(),
                ));
            }
            let channel_id = i64_channel(pending.channel_id)?;
            let identity = CompletionDeliveryIdentity::outbox(&pending.delivery_id);
            self.send_idempotent_text(
                channel_id,
                &identity,
                &pending.content,
                &cdr_store::new_reply::DeliveryGuard {
                    job_id: &pending.job_id,
                    thread_id: &pending.target_thread_id,
                    turn_id: &pending.turn_id,
                },
            )
            .await
        }
        .await;
        if let Err(error) = result {
            if matches!(error, CompletionWorkerError::Held(_)) {
                return Err(error);
            }
            let failure_time = now().map_err(|time_error| {
                eprintln!(
                    "completion_pending_delivery_error delivery_id={} error={error}; failure_record_error={time_error}",
                    pending.delivery_id
                );
                time_error
            })?;
            if let Err(record_error) = record_failure(
                self.queue.db_path(),
                &pending.delivery_id,
                &error.to_string(),
                failure_time,
            ) {
                eprintln!(
                    "completion_pending_delivery_error delivery_id={} error={error}; failure_record_error={record_error}",
                    pending.delivery_id,
                );
                return Err(record_error.into());
            }
            return Err(error);
        }
        let _ = complete(self.queue.db_path(), &pending.delivery_id)?;
        Ok(())
    }

    pub(super) async fn send_idempotent_text(
        &self,
        channel_id: Id<ChannelMarker>,
        identity: &CompletionDeliveryIdentity,
        text: &str,
        guard: &cdr_store::new_reply::DeliveryGuard<'_>,
    ) -> Result<(), CompletionWorkerError> {
        deliver_idempotent_chunks(
            text,
            &DeliveryPolicy {
                retry_delays: Vec::new(),
                chunk_markers: true,
            },
            identity,
            |chunk| async move {
                super::receipt::send_chunk_guarded(
                    self.queue.db_path(),
                    &self.http,
                    channel_id,
                    &chunk,
                    &[],
                    Some(guard),
                )
                .await
            },
        )
        .await
        .map_err(|error| {
            if matches!(error.source, CompletionWorkerError::Held(_)) {
                error.source
            } else {
                CompletionWorkerError::Delivery(format!("{error:?}"))
            }
        })?;
        Ok(())
    }
}

async fn attempt_all<I, E, F, Fut>(
    items: impl IntoIterator<Item = I>,
    mut attempt: F,
) -> Result<(), E>
where
    F: FnMut(I) -> Fut,
    Fut: std::future::Future<Output = Result<(), E>>,
{
    let mut first_error = None;
    for item in items {
        if let Err(error) = attempt(item).await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[must_use]
pub(super) fn commentary_message(text: &str) -> String {
    format!("In progress\n{text}")
}

#[must_use]
pub fn completion_message(
    completion: &TurnCompletion,
    exact: &str,
    goal: Option<ThreadGoalStatus>,
) -> String {
    let base = match completion.status {
        TurnStatus::Completed => {
            if exact.is_empty() {
                format!("Final\n{NO_VISIBLE_REPLY}")
            } else {
                format!("Final\n{exact}")
            }
        }
        TurnStatus::Interrupted => "Interrupted\nCodex turn was interrupted.".to_owned(),
        TurnStatus::Failed => {
            if completion.error_message.is_empty() {
                "Failed\nCodex turn failed without an error message.".to_owned()
            } else {
                format!(
                    "Failed\n{}",
                    crate::error_message::readable_error(&completion.error_message)
                )
            }
        }
        TurnStatus::InProgress => "In progress\nmessage: Codex turn is still running.".to_owned(),
    };
    goal.filter(|status| *status != ThreadGoalStatus::Complete)
        .map_or_else(
            || base.clone(),
            |status| format!("[Goal status: {}]\n{base}", goal_name(status)),
        )
}

const fn goal_name(status: ThreadGoalStatus) -> &'static str {
    match status {
        ThreadGoalStatus::Active => "active",
        ThreadGoalStatus::Paused => "paused",
        ThreadGoalStatus::Blocked => "blocked",
        ThreadGoalStatus::UsageLimited => "usage-limited",
        ThreadGoalStatus::BudgetLimited => "budget-limited",
        ThreadGoalStatus::Complete => "complete",
    }
}

pub(super) fn i64_channel(value: i64) -> Result<Id<ChannelMarker>, CompletionWorkerError> {
    u64::try_from(value)
        .ok()
        .and_then(Id::new_checked)
        .ok_or(CompletionWorkerError::ChannelId)
}

fn now() -> Result<f64, StoreError> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64())
}

impl CompletionWorker {
    pub(super) fn report(result: Result<(), CompletionWorkerError>) {
        if let Err(error) = result {
            eprintln!("completion_worker_error: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn commentary_has_only_a_heading_not_another_request_echo() {
        assert_eq!(
            commentary_message("실제 진행 내용"),
            "In progress\n실제 진행 내용"
        );
    }

    #[test]
    fn successful_reply_has_explicit_final_heading() {
        let completion = TurnCompletion {
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            status: TurnStatus::Completed,
            error_message: String::new(),
            interrupt_origin: None,
            duration_ms: None,
        };
        assert_eq!(completion_message(&completion, "답변", None), "Final\n답변");
    }

    #[tokio::test]
    async fn pending_batch_attempts_later_items_before_returning_the_first_error() {
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&attempts);

        let error = attempt_all(["first", "second"], move |delivery_id| {
            recorded.lock().unwrap().push(delivery_id);
            async move {
                if delivery_id == "first" {
                    Err("persistent Discord failure")
                } else {
                    Ok(())
                }
            }
        })
        .await
        .unwrap_err();

        assert_eq!(error, "persistent Discord failure");
        assert_eq!(attempts.lock().unwrap().as_slice(), &["first", "second"]);
    }
}
