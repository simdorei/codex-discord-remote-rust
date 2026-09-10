use super::{CompletionWorker, CompletionWorkerError, history_request::full_history_request};
use cdr_app_server::goal::{ThreadGoalStatus, parse_thread_goal_status};
use cdr_app_server::outcomes::TurnOutcomeError;
use cdr_app_server::outcomes::{TurnCompletion, extract_turn_final_text};
use cdr_app_server::requests::get_goal;
use std::time::Duration;

const HISTORY_ATTEMPTS: usize = 3;
const HISTORY_RETRY_DELAY: Duration = Duration::from_millis(100);

impl CompletionWorker {
    pub(super) async fn goal_status(
        &self,
        generation: u64,
        thread_id: &str,
    ) -> Result<Option<ThreadGoalStatus>, CompletionWorkerError> {
        let result = self
            .server
            .execute(get_goal(thread_id), Some(generation))
            .await?;
        parse_thread_goal_status(&result, thread_id)
            .map_err(|error| CompletionWorkerError::Goal(error.to_string()))
    }

    pub(super) async fn exact_text(
        &self,
        server_generation: u64,
        evidence_generation: i64,
        completion: &TurnCompletion,
    ) -> Result<String, CompletionWorkerError> {
        for attempt in 0..HISTORY_ATTEMPTS {
            let result = self
                .server
                .execute(
                    full_history_request(&completion.thread_id, self.history_read_timeout),
                    Some(server_generation),
                )
                .await?;
            match extract_turn_final_text(&result, &completion.thread_id, &completion.turn_id) {
                Ok(text) => return Ok(text),
                Err(TurnOutcomeError::TurnNotFound) if attempt + 1 < HISTORY_ATTEMPTS => {
                    tokio::time::sleep(HISTORY_RETRY_DELAY).await;
                }
                Err(TurnOutcomeError::TurnNotFound) => break,
                Err(error) => return Err(error.into()),
            }
        }
        if let Some(text) = cdr_store::observed_final_answer::get(
            self.queue.db_path(),
            &completion.thread_id,
            &completion.turn_id,
            evidence_generation,
        )? {
            return Ok(text);
        }
        Err(TurnOutcomeError::TurnNotFound.into())
    }
}
