use cdr_app_server::outcomes::{
    TurnStatus, extract_completed_final_answer, extract_turn_final_text, parse_thread_turn_states,
    parse_turn_completion,
};
use serde_json::json;

#[test]
fn terminal_notification_parses_status_duration_and_failed_error() {
    let completion = parse_turn_completion(
        &json!({
            "threadId":"thread-a",
            "turn":{"id":"turn-a", "status":"failed", "durationMs":123, "error":{"message":" model exited "}}
        }),
        true,
    )
    .unwrap();

    assert_eq!(completion.thread_id, "thread-a");
    assert_eq!(completion.turn_id, "turn-a");
    assert_eq!(completion.status, TurnStatus::Failed);
    assert_eq!(completion.duration_ms, Some(123));
    assert_eq!(completion.error_message, "model exited");
}

#[test]
fn item_completed_final_answer_keeps_exact_turn_identity_and_text() {
    let answer = extract_completed_final_answer(&serde_json::json!({
        "threadId": "thread-a",
        "turnId": "turn-a",
        "item": {
            "id": "item-a",
            "type": "agentMessage",
            "phase": "final_answer",
            "text": "exact final\nanswer"
        }
    }))
    .unwrap();

    assert_eq!(answer.thread_id, "thread-a");
    assert_eq!(answer.turn_id, "turn-a");
    assert_eq!(answer.text, "exact final\nanswer");
}

#[test]
fn item_completed_commentary_is_not_a_final_answer() {
    assert!(
        extract_completed_final_answer(&serde_json::json!({
            "threadId": "thread-a",
            "turnId": "turn-a",
            "item": {
                "id": "item-a",
                "type": "agentMessage",
                "phase": "commentary",
                "text": "still working"
            }
        }))
        .is_none()
    );
}

#[test]
fn in_progress_is_rejected_for_completion_but_included_in_thread_states() {
    let payload = json!({"threadId":"thread-a", "turn":{"id":"turn-a", "status":"inProgress"}});
    assert!(parse_turn_completion(&payload, false).is_err());

    let states = parse_thread_turn_states(
        &json!({"thread":{"id":"thread-a", "turns":[{"id":"turn-a", "status":"inProgress"}]}}),
        "thread-a",
    )
    .unwrap();
    assert_eq!(states["turn-a"].status, TurnStatus::InProgress);
}

#[test]
fn thread_identity_and_shapes_fail_closed() {
    let wrong =
        parse_thread_turn_states(&json!({"thread":{"id":"thread-b", "turns":[]}}), "thread-a")
            .unwrap_err();
    assert!(wrong.to_string().contains("different thread"));

    let malformed = parse_thread_turn_states(
        &json!({"thread":{"id":"thread-a", "turns":[{"id":"turn-a"}]}}),
        "thread-a",
    )
    .unwrap_err();
    assert!(malformed.to_string().contains("no status"));
}

#[test]
fn errors_are_only_exposed_for_failed_turns_and_are_bounded() {
    let long = "x".repeat(1_100);
    let states = parse_thread_turn_states(
        &json!({"thread":{"id":"thread-a", "turns":[
            {"id":"done", "status":"completed", "error":{"message":"ignore"}},
            {"id":"failed", "status":"failed", "error":{"message":long}}
        ]}}),
        "thread-a",
    )
    .unwrap();

    assert!(states["done"].error_message.is_empty());
    assert_eq!(states["failed"].error_message.len(), 1_000);
}

#[test]
fn null_error_is_treated_as_no_error_for_current_app_server_payloads() {
    let completion = parse_turn_completion(
        &json!({
            "threadId":"thread-a",
            "turn":{"id":"turn-a", "status":"completed", "error":null}
        }),
        false,
    )
    .unwrap();

    assert_eq!(completion.status, TurnStatus::Completed);
    assert!(completion.error_message.is_empty());
}

#[test]
fn exact_final_text_is_read_from_the_requested_thread_and_turn() {
    let result = json!({"thread":{"id":"thread-a", "turns":[
        {"id":"old", "status":"completed", "items":[{"type":"agentMessage","text":"stale"}]},
        {"id":"turn-a", "status":"completed", "items":[
            {"type":"agentMessage","text":"commentary","phase":"commentary"},
            {"type":"agentMessage","text":"final answer","phase":"final_answer"}
        ]}
    ]}});

    assert_eq!(
        extract_turn_final_text(&result, "thread-a", "turn-a").unwrap(),
        "final answer"
    );
    assert!(extract_turn_final_text(&result, "thread-a", "missing").is_err());
    assert!(extract_turn_final_text(&result, "wrong", "turn-a").is_err());
}

#[test]
fn final_text_supports_current_content_blocks_and_last_agent_fallback() {
    let result = json!({"thread":{"id":"thread-a", "turns":[{
        "id":"turn-a", "status":"completed", "items":[
            {"type":"agentMessage","content":[{"type":"output_text","text":"first"}]},
            {"type":"agentMessage","text":"last"}
        ]
    }]}});
    assert_eq!(
        extract_turn_final_text(&result, "thread-a", "turn-a").unwrap(),
        "last"
    );
}
