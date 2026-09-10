use cdr_app_server::goal::ThreadGoalStatus;
use cdr_app_server::outcomes::{InterruptOrigin, TurnCompletion, TurnStatus};
use cdr_runtime::final_watch::{
    FinalWatchState, GoalLookup, NativeObservation, WatchStatus, process_events, reconcile_native,
};
use serde_json::json;

fn completion(status: TurnStatus) -> TurnCompletion {
    TurnCompletion {
        thread_id: "thread-a".into(),
        turn_id: "turn-a".into(),
        status,
        error_message: String::new(),
        interrupt_origin: None,
        duration_ms: None,
    }
}

#[test]
fn rollout_deduplicates_commentary_and_finishes_parent_turn() {
    let mut state = FinalWatchState::default();
    let result = process_events(
        &mut state,
        &[
            json!({"type":"event_msg", "payload":{"type":"agent_message", "message":"checking"}}),
            json!({"type":"event_msg", "payload":{"type":"agent_message", "message":"checking"}}),
            json!({"type":"response_item", "payload":{"type":"message", "role":"assistant", "phase":"commentary", "content":[{"type":"output_text", "text":"thinking"}]}}),
            json!({"type":"event_msg", "payload":{"type":"task_complete", "last_agent_message":"done"}}),
        ],
        None,
        &GoalLookup::Absent,
    )
    .unwrap();

    assert_eq!(result.status, WatchStatus::Final);
    assert_eq!(result.commentary, vec!["checking", "thinking"]);
    assert_eq!(result.final_answer, "done");
}

#[test]
fn active_goal_turn_is_progress_not_a_false_final() {
    let mut state = FinalWatchState::default();
    let result = process_events(
        &mut state,
        &[json!({"type":"event_msg", "payload":{"type":"task_complete", "last_agent_message":"still working"}})],
        None,
        &GoalLookup::Present(ThreadGoalStatus::Active),
    )
    .unwrap();

    assert_eq!(result.status, WatchStatus::Progress);
    assert_eq!(result.commentary, vec!["still working"]);
    assert!(result.final_answer.is_empty());
}

#[test]
fn expected_turn_ignores_stale_terminal_then_reconciles_exact_native_completion() {
    let mut state = FinalWatchState::default();
    assert!(
        process_events(
            &mut state,
            &[
                json!({"type":"event_msg", "payload":{"type":"task_complete", "turn_id":"old", "last_agent_message":"old"}}),
                json!({"type":"event_msg", "payload":{"type":"task_complete", "turn_id":"turn-a", "last_agent_message":"new"}}),
            ],
            Some("turn-a"),
            &GoalLookup::Absent,
        )
        .is_none()
    );
    let result = reconcile_native(
        &mut state,
        &NativeObservation::Found(completion(TurnStatus::Completed)),
        &GoalLookup::Absent,
        false,
    )
    .unwrap();
    assert_eq!(result.status, WatchStatus::Final);
    assert_eq!(result.final_answer, "new");
}

#[test]
fn native_failure_and_remote_interrupt_keep_terminal_reason() {
    let mut state = FinalWatchState::default();
    let mut failed = completion(TurnStatus::Failed);
    failed.error_message = "model exited".into();
    let result = reconcile_native(
        &mut state,
        &NativeObservation::Found(failed),
        &GoalLookup::Absent,
        true,
    )
    .unwrap();
    assert_eq!(result.status, WatchStatus::Failed);
    assert_eq!(result.error_message, "model exited");

    let mut state = FinalWatchState::default();
    let mut interrupted = completion(TurnStatus::Interrupted);
    interrupted.interrupt_origin = Some(InterruptOrigin::RemoteUserIntent);
    let result = reconcile_native(
        &mut state,
        &NativeObservation::Found(interrupted),
        &GoalLookup::Absent,
        true,
    )
    .unwrap();
    assert_eq!(result.status, WatchStatus::Aborted);
    assert_eq!(
        result.interrupt_origin.as_deref(),
        Some("remote_user_intent")
    );
}

#[test]
fn rejected_approval_and_reconcile_timeout_are_visible() {
    let mut state = FinalWatchState::default();
    let result = process_events(
        &mut state,
        &[
            json!({"type":"response_item", "payload":{"type":"function_call_output", "output":"Rejected by user: no"}}),
            json!({"type":"event_msg", "payload":{"type":"task_complete", "turn_id":"turn-a"}}),
        ],
        Some("turn-a"),
        &GoalLookup::Absent,
    );
    assert!(result.is_none());
    assert_eq!(
        state.commentary,
        vec!["[approval_rejected]\nCommand approval was rejected by user."]
    );

    let result = reconcile_native(
        &mut state,
        &NativeObservation::Pending,
        &GoalLookup::Absent,
        true,
    )
    .unwrap();
    assert_eq!(result.status, WatchStatus::TransportError);
    assert!(result.error_message.contains("Timed out reconciling"));
}
