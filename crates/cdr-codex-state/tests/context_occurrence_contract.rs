use cdr_codex_state::{ContextReadBudget, RecentTextMode, read_context_snapshot_with_mode};
use serde_json::json;

#[test]
fn distinct_equal_finals_remain_in_order_and_unidentified_dual_records_are_not_guessed_equal() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rollout.jsonl");
    let mut events = vec![json!({"type":"session_meta","payload":{"id":"original"}})];
    for (turn, text) in [("t1", "승인"), ("t2", "수정"), ("t3", "승인")] {
        events.push(json!({"type":"event_msg","payload":{"type":"agent_message","phase":"final","turn_id":turn,"message":text}}));
    }
    std::fs::write(
        &path,
        events
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();
    let snapshot = read_context_snapshot_with_mode(
        &path,
        "original",
        ContextReadBudget::default(),
        5,
        RecentTextMode::UserAndFinal,
    )
    .unwrap();
    assert_eq!(
        snapshot
            .recent_items
            .iter()
            .map(|v| v.text.as_str())
            .collect::<Vec<_>>(),
        ["승인", "수정", "승인"]
    );
    events.push(json!({"type":"response_item","payload":{"type":"message","role":"assistant","phase":"final","content":[{"type":"output_text","text":"승인"}]}}));
    std::fs::write(
        &path,
        events
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();
    let snapshot = read_context_snapshot_with_mode(
        &path,
        "original",
        ContextReadBudget::default(),
        3,
        RecentTextMode::UserAndFinal,
    )
    .unwrap();
    assert_eq!(
        snapshot
            .recent_items
            .iter()
            .map(|v| v.text.as_str())
            .collect::<Vec<_>>(),
        ["수정", "승인", "승인"]
    );
}
