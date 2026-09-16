use super::{CompletionWorker, CompletionWorkerError, history_request::full_history_request};
use cdr_app_server::goal::{ThreadGoalStatus, parse_thread_goal_status};
use cdr_app_server::outcomes::{TurnCompletion, TurnOutcomeError, extract_turn_text};
use cdr_app_server::requests::get_goal;
use cdr_store::queue::StoredQueueJob;
use std::time::Duration;

const HISTORY_ATTEMPTS: usize = 3;
const HISTORY_RETRY_DELAY: Duration = Duration::from_millis(100);

#[derive(Default)]
pub(super) struct ExactReply {
    pub text: Option<String>,
    pub needs_goal_handoff: bool,
}

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
        owner: &StoredQueueJob,
        inspect_goal_history: bool,
    ) -> Result<ExactReply, CompletionWorkerError> {
        let mut reply = ExactReply::default();
        for attempt in 0..HISTORY_ATTEMPTS {
            let result = self
                .server
                .execute(
                    full_history_request(&completion.thread_id, self.history_read_timeout),
                    Some(server_generation),
                )
                .await?;
            if inspect_goal_history {
                // Once observed, a successor cannot disappear as authority to
                // finalize this older turn merely because a later read is sparse.
                reply.needs_goal_handoff |=
                    self.goal_history_has_unattached_turn(&result, owner, completion)?;
                if owner.goal_waiting && reply.needs_goal_handoff {
                    return Err(CompletionWorkerError::Held(
                        "unattached turn requires an exact start observation; waiting owner retained".into(),
                    ));
                }
            }
            match extract_turn_text(&result, &completion.thread_id, &completion.turn_id) {
                Ok(observed) if observed.explicit_final => {
                    reply.text = Some(observed.text);
                    break;
                }
                Ok(observed) => reply.text = Some(observed.text),
                Err(TurnOutcomeError::TurnNotFound) => {}
                Err(error) => return Err(error.into()),
            }
            // Exact final-answer evidence outranks empty/commentary/legacy text,
            // not an explicit history final. Never borrow a different generation.
            if let Some(text) = cdr_store::observed_final_answer::get(
                self.queue.db_path(),
                &completion.thread_id,
                &completion.turn_id,
                evidence_generation,
            )? {
                reply.text = Some(text);
                break;
            }
            if attempt + 1 < HISTORY_ATTEMPTS {
                tokio::time::sleep(HISTORY_RETRY_DELAY).await;
            }
        }
        self.require_completion_owner(server_generation, owner)?;
        Ok(reply)
    }
}
