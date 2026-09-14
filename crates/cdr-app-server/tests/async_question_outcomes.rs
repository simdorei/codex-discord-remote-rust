use cdr_app_server::outcomes::{extract_completed_final_answer, extract_turn_final_text};
use serde_json::{Value, json};

fn question(questions: Value) -> Value {
    json!({"id":"question-item", "type":"agentMessage", "phase":"final_answer",
        "delivery":"async", "text":"어떤 방법으로 할까요?", "questions":questions})
}

#[test]
fn async_question_is_never_live_final_evidence_even_when_unrenderable() {
    for questions in [
        json!([{"title":"방법", "options":["A", "B"]}]),
        json!([]),
        Value::Null,
    ] {
        let event = json!({"threadId":"thread", "turnId":"turn", "item":question(questions)});
        assert_eq!(extract_completed_final_answer(&event), None);
    }
}

#[test]
fn async_question_is_never_history_final_or_last_agent_fallback() {
    for phase in [json!("final_answer"), json!("commentary"), Value::Null] {
        let mut item = question(json!([]));
        item["phase"] = phase;
        let history = json!({"thread":{"id":"thread", "turns":[{"id":"turn", "items":[item]}]}});
        assert_eq!(
            extract_turn_final_text(&history, "thread", "turn").unwrap(),
            ""
        );
    }
}

#[test]
fn real_final_survives_late_async_question() {
    let history = json!({"thread":{"id":"thread", "turns":[{"id":"turn", "items":[
        {"type":"agentMessage", "phase":"final_answer", "text":"실제 최종 답변"},
        question(json!([{"title":"다음 작업?", "options":["A", "B"]}]))
    ]}]}});
    assert_eq!(
        extract_turn_final_text(&history, "thread", "turn").unwrap(),
        "실제 최종 답변"
    );
}
