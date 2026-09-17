use cdr_app_server::goal::ThreadGoalStatus;
use cdr_app_server::outcomes::{TurnCompletion, TurnStatus};
use cdr_runtime::completion_worker::completion_message;

fn completion(status: TurnStatus, error: &str) -> TurnCompletion {
    TurnCompletion {
        thread_id: "thread-a".into(),
        turn_id: "turn-a".into(),
        status,
        error_message: error.into(),
        interrupt_origin: None,
        duration_ms: None,
        usage_limit: false,
    }
}

#[test]
fn exact_completed_answer_and_empty_fallback_are_visible() {
    assert_eq!(
        completion_message(&completion(TurnStatus::Completed, ""), "answer", None),
        "Final\nanswer"
    );
    assert_eq!(
        completion_message(&completion(TurnStatus::Completed, ""), "", None),
        "Final\nCompleted (no visible reply)"
    );
}

#[test]
fn failed_interrupted_and_noncomplete_goal_statuses_are_explicit() {
    assert_eq!(
        completion_message(&completion(TurnStatus::Failed, "model exited"), "", None),
        "Failed\nmodel exited"
    );
    assert_eq!(
        completion_message(&completion(TurnStatus::Interrupted, ""), "", None),
        "Interrupted\nCodex turn was interrupted."
    );
    assert_eq!(
        completion_message(
            &completion(TurnStatus::Completed, ""),
            "partial",
            Some(ThreadGoalStatus::Blocked),
        ),
        "[Goal status: blocked]\nFinal\npartial"
    );
}

#[test]
fn nested_server_error_is_readable_without_losing_the_actual_message() {
    let error = r#"{"type":"error","status":400,"error":{"type":"invalid_request_error","message":"The 'GPT-6-astra' model is not supported when using Codex with a ChatGPT account."}}"#;
    assert_eq!(
        completion_message(&completion(TurnStatus::Failed, error), "", None),
        "Failed\nThe 'GPT-6-astra' model is not supported when using Codex with a ChatGPT account."
    );
}
