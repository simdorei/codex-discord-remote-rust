use cdr_codex_state::{ContextReadBudget, read_context_snapshot};
use serde_json::{Value, json};

fn write(path: &std::path::Path, events: &[Value]) {
    let mut lines = vec![json!({"type":"session_meta","payload":{"id":"original"}}).to_string()];
    lines.extend(events.iter().map(Value::to_string));
    std::fs::write(path, format!("{}\n", lines.join("\n"))).unwrap();
}

#[test]
fn recent_visible_text_is_bounded_deduplicated_and_refreshed_from_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rollout.jsonl");
    let mut events = vec![
        json!({"type":"event_msg","payload":{"type":"user_message","message":"요청입니다"}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"요청입니다"}]}}),
        json!({"type":"response_item","payload":{"type":"message","role":"assistant","phase":"analysis","content":[{"type":"output_text","text":"private reasoning"}]}}),
        json!({"type":"response_item","payload":{"type":"function_call_output","output":"tool-private"}}),
        json!({"type":"response_item","payload":{"type":"message","role":"system","content":[{"type":"input_text","text":"system-private"}]}}),
        json!({"type":"event_msg","payload":{"type":"agent_message","phase":"final_answer","message":"답변입니다"}}),
    ];
    write(&path, &events);
    let first = read_context_snapshot(&path, "original", ContextReadBudget::default(), 2).unwrap();
    assert_eq!(
        first
            .recent_items
            .iter()
            .map(|v| (v.label, v.text.as_str()))
            .collect::<Vec<_>>(),
        vec![("user", "요청입니다"), ("assistant final", "답변입니다")]
    );
    assert!(first.usage.is_none());
    events
        .push(json!({"type":"event_msg","payload":{"type":"user_message","message":"다음 요청"}}));
    write(&path, &events);
    let refreshed =
        read_context_snapshot(&path, "original", ContextReadBudget::default(), 2).unwrap();
    assert_eq!(
        refreshed
            .recent_items
            .iter()
            .map(|v| v.text.as_str())
            .collect::<Vec<_>>(),
        vec!["답변입니다", "다음 요청"]
    );
}

#[test]
fn truncation_is_utf8_safe_and_interactive_notices_exclude_raw_arguments() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rollout.jsonl");
    write(
        &path,
        &[
            json!({"type":"event_msg","payload":{"type":"user_message","message":"한".repeat(2000)}}),
            json!({"type":"response_item","payload":{"type":"function_call","name":"request_user_input","arguments":"private arguments"}}),
            json!({"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":json!({"sandbox_permissions":"require_escalated","cmd":"private command"}).to_string()}}),
        ],
    );
    let result = read_context_snapshot(&path, "original", ContextReadBudget::default(), 3).unwrap();
    assert_eq!(result.recent_items.len(), 3);
    assert_eq!(result.recent_items[0].text, "한".repeat(1500));
    assert!(result.recent_items[0].truncated);
    assert_eq!(result.recent_items[1].text, "[choice_required]");
    assert_eq!(result.recent_items[2].text, "[approval_required]");
}
