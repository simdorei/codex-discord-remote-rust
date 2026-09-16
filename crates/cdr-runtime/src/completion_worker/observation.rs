use super::{CompletionWorker, CompletionWorkerError};
use cdr_app_server::outcomes::completion_journal_payload;
use cdr_app_server::{
    ResidentNotificationEvent,
    outcomes::{extract_completed_final_answer, parse_turn_completion},
};
use cdr_store::{observed_completion, observed_final_answer};

impl CompletionWorker {
    pub(super) fn observe_terminal(
        &self,
        event: &ResidentNotificationEvent,
    ) -> Result<(), CompletionWorkerError> {
        let ResidentNotificationEvent::Notification {
            generation,
            notification,
        } = event
        else {
            return Ok(());
        };
        let generation = i64::try_from(*generation)
            .map_err(|_| crate::queue_runner::QueueRunnerError::IntegerRange)?;
        if notification.method == "item/completed" {
            if let Some(answer) = extract_completed_final_answer(&notification.params)
                && observed_final_answer::record(
                    self.queue.db_path(),
                    &answer.thread_id,
                    &answer.turn_id,
                    generation,
                    &answer.text,
                )?
            {
                eprintln!(
                    "completion_final_observed thread_id={} turn_id={} generation={generation}",
                    answer.thread_id, answer.turn_id
                );
            }
            return Ok(());
        }
        if notification.method != "turn/completed" {
            return Ok(());
        }
        let completion = parse_turn_completion(&notification.params, false)?;
        // Store only completion metadata, not prompt, tools or conversation items.
        let payload = completion_journal_payload(&completion).to_string();
        if observed_completion::record(
            self.queue.db_path(),
            &completion.thread_id,
            &completion.turn_id,
            generation,
            &payload,
        )? {
            eprintln!(
                "completion_observed thread_id={} turn_id={} generation={generation}",
                completion.thread_id, completion.turn_id
            );
        }
        Ok(())
    }

    pub(super) async fn recover_observed(&self) -> Result<(), CompletionWorkerError> {
        let mut first_error = None;
        for (thread, turn, generation, payload) in
            observed_completion::pending_with_generation(self.queue.db_path())?
        {
            let result = async {
                let value = serde_json::from_str(&payload).map_err(|error| {
                    CompletionWorkerError::Delivery(format!("invalid terminal journal: {error}"))
                })?;
                let completion = parse_turn_completion(&value, false)?;
                self.finish(self.server.generation(), generation, &completion)
                    .await
            }
            .await;
            if let Err(error) = result {
                observed_completion::record_error(
                    self.queue.db_path(),
                    &thread,
                    &turn,
                    &error.to_string(),
                )?;
                eprintln!(
                    "completion_observation_pending thread_id={thread} turn_id={turn} error={error}"
                );
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}
