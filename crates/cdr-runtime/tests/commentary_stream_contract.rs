use cdr_runtime::commentary_stream::CommentaryBuffer;
use serde_json::json;

#[test]
fn reasoning_summaries_do_not_create_extra_progress_messages() {
    let mut buffer = CommentaryBuffer::default();
    let first = json!({
        "threadId": "thread-a",
        "turnId": "turn-a",
        "itemId": "item-a",
        "summaryIndex": 0,
        "delta": "Checking "
    });
    let second = json!({
        "threadId": "thread-a",
        "turnId": "turn-a",
        "itemId": "item-a",
        "summaryIndex": 0,
        "delta": "the rollback path."
    });
    assert!(
        buffer
            .observe("item/reasoning/summaryTextDelta", &first)
            .is_none()
    );
    assert!(
        buffer
            .observe("item/reasoning/summaryTextDelta", &second)
            .is_none()
    );

    let completed = json!({
        "threadId": "thread-a",
        "turnId": "turn-a",
        "item": {"id": "item-a", "type": "reasoning", "summary": []}
    });
    assert!(buffer.observe("item/completed", &completed).is_none());
    assert!(buffer.observe("item/completed", &completed).is_none());
}

#[test]
fn raw_reasoning_and_agent_answer_deltas_are_not_exposed_as_commentary() {
    let mut buffer = CommentaryBuffer::default();
    let params = json!({
        "threadId": "thread-a",
        "turnId": "turn-a",
        "itemId": "item-a",
        "delta": "private or final text"
    });

    assert!(
        buffer
            .observe("item/reasoning/textDelta", &params)
            .is_none()
    );
    assert!(buffer.observe("item/agentMessage/delta", &params).is_none());
    assert_eq!(buffer.active_items(), 0);
}
