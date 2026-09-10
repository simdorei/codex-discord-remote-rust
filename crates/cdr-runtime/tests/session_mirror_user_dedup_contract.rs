use cdr_runtime::session_mirror::{MirrorDetail, MirrorKind, collect_items};
use serde_json::{Map, Value, json};

fn event(mut value: Value) -> Map<String, Value> {
    std::mem::take(value.as_object_mut().expect("event object"))
}

#[test]
fn equivalent_user_shapes_mirror_once_but_a_later_repeat_is_preserved() {
    let items = collect_items(
        "thread-1",
        &[
            event(json!({
                "timestamp": "2026-09-04T04:35:40.108Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "not work .."}]
                }
            })),
            event(json!({
                "timestamp": "2026-09-04T04:35:40.108Z",
                "type": "event_msg",
                "payload": {"type": "user_message", "message": "not work .."}
            })),
            event(json!({
                "timestamp": "2026-09-04T04:36:40.108Z",
                "type": "event_msg",
                "payload": {"type": "user_message", "message": "not work .."}
            })),
        ],
        MirrorDetail::Send,
    );

    assert_eq!(
        items.len(),
        2,
        "one user occurrence must mirror exactly once"
    );
    assert!(items.iter().all(|item| item.kind == MirrorKind::User));
    assert_eq!(items[0].text, "not work ..");
    assert_eq!(items[1].text, "not work ..");
    assert_ne!(
        items[0].digest, items[1].digest,
        "a later intentional repeat must retain a distinct identity"
    );
}
