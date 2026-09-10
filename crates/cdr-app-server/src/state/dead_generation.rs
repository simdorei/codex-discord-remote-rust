use std::cmp::Ordering;

use super::RuntimeState;
use crate::{DeadActiveTurn, DeadGenerationWork, DeadServerRequest, RequestId, ServerRequest};

impl RuntimeState {
    pub(crate) fn dead_generation_work(&self, generation: u64) -> Option<DeadGenerationWork> {
        let closed_reason = self.closed_reason.clone()?;
        let mut active_turns = self
            .active_turns
            .iter()
            .map(|(thread_id, turn_id)| DeadActiveTurn {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
            })
            .collect::<Vec<_>>();
        active_turns.sort_by(|left, right| {
            left.thread_id
                .cmp(&right.thread_id)
                .then_with(|| left.turn_id.cmp(&right.turn_id))
        });

        let mut server_requests = self
            .server_requests
            .unsettled(None)
            .iter()
            .map(request_identity)
            .collect::<Vec<_>>();
        server_requests.sort_by(compare_request_identity);
        Some(DeadGenerationWork {
            generation,
            closed_reason,
            active_turns,
            server_requests,
        })
    }

    pub(crate) fn settle_dead_generation_after_exact_match(&mut self) {
        self.active_turns.clear();
        self.server_requests
            .settle_dead_generation_after_exact_match();
    }
}

fn request_identity(request: &ServerRequest) -> DeadServerRequest {
    DeadServerRequest {
        id: request.id.clone(),
        occurrence: request.occurrence,
        method: request.method.clone(),
        params: request.params.clone(),
    }
}

fn compare_request_identity(left: &DeadServerRequest, right: &DeadServerRequest) -> Ordering {
    compare_request_id(&left.id, &right.id)
        .then_with(|| left.occurrence.as_bytes().cmp(right.occurrence.as_bytes()))
}

fn compare_request_id(left: &RequestId, right: &RequestId) -> Ordering {
    match (left, right) {
        (RequestId::Integer(left), RequestId::Integer(right)) => left.cmp(right),
        (RequestId::String(left), RequestId::String(right)) => left.cmp(right),
        (RequestId::Integer(_), RequestId::String(_)) => Ordering::Less,
        (RequestId::String(_), RequestId::Integer(_)) => Ordering::Greater,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::RuntimeState;
    use crate::{Notification, RequestId, ServerRequest, ServerRequestOccurrence};

    fn request(
        id: RequestId,
        occurrence: u128,
        method: &str,
        thread_id: &str,
        marker: &str,
    ) -> ServerRequest {
        ServerRequest {
            id,
            occurrence: ServerRequestOccurrence::from_bytes(occurrence.to_be_bytes()),
            method: method.to_owned(),
            params: json!({"threadId":thread_id, "turnId":"turn-a", "marker":marker}),
        }
    }

    #[test]
    fn snapshot_sorts_active_turns_and_includes_pending_claimed_and_deferred_requests() {
        let mut state = RuntimeState::starting(None);
        state.closed_reason = Some("fixture closed".to_owned());
        for (thread_id, turn_id) in [("thread-z", "turn-a"), ("thread-a", "turn-z")] {
            state.record_notification(Notification {
                method: "turn/started".to_owned(),
                params: json!({"threadId":thread_id, "turn":{"id":turn_id}}),
            });
        }
        let pending = request(
            RequestId::String("z".to_owned()),
            3,
            "item/tool/requestUserInput",
            "thread-pending",
            "pending-input",
        );
        let claimed = request(
            RequestId::Integer(2),
            2,
            "item/commandExecution/requestApproval",
            "thread-claimed",
            "claimed-approval",
        );
        let deferred = request(
            RequestId::Integer(2),
            1,
            "item/tool/requestUserInput",
            "thread-deferred",
            "deferred-input",
        );
        state
            .record_server_request(pending.clone())
            .expect("pending request");
        state
            .record_server_request(claimed.clone())
            .expect("claimed request");
        state
            .begin_server_response(&claimed.id, claimed.occurrence)
            .expect("claim request");
        state
            .record_server_request(deferred.clone())
            .expect("defer reused id");

        let work = state.dead_generation_work(7).expect("dead work");
        assert_eq!(work.generation, 7);
        assert_eq!(work.closed_reason, "fixture closed");
        assert_eq!(
            work.active_turns
                .iter()
                .map(|turn| (turn.thread_id.as_str(), turn.turn_id.as_str()))
                .collect::<Vec<_>>(),
            vec![("thread-a", "turn-z"), ("thread-z", "turn-a")]
        );
        assert_eq!(
            work.server_requests
                .iter()
                .map(|request| {
                    (
                        &request.id,
                        request.occurrence,
                        request.method.as_str(),
                        request.params.clone(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (
                    &deferred.id,
                    deferred.occurrence,
                    deferred.method.as_str(),
                    deferred.params.clone(),
                ),
                (
                    &claimed.id,
                    claimed.occurrence,
                    claimed.method.as_str(),
                    claimed.params.clone(),
                ),
                (
                    &pending.id,
                    pending.occurrence,
                    pending.method.as_str(),
                    pending.params.clone(),
                ),
            ]
        );
    }
}
