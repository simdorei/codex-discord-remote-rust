use cdr_runtime::session_mirror::{MirrorDetail, collect_items};
use cdr_runtime::session_mirror_worker::{
    SESSION_MIRROR_ASSISTANT_TEXT_NONCE_DOMAIN, SESSION_MIRROR_EVENT_NONCE_DOMAIN,
    session_delivery_identity,
};
use serde_json::{Map, Value, json};

fn event(mut value: Value) -> Map<String, Value> {
    std::mem::take(value.as_object_mut().unwrap())
}

#[test]
fn final_identity_is_per_turn_not_per_text_or_observation_timestamp() {
    let collect = |turn: &str, time: &str| {
        collect_items("thread",&[event(json!({
        "timestamp":time,"type":"event_msg","payload":{"type":"task_complete","turn_id":turn,"last_agent_message":"same reply"}
    }))],MirrorDetail::Send).remove(0)
    };
    let one = collect("one", "1");
    let repeated = collect("one", "2");
    let two = collect("two", "3");
    assert_eq!(one.digest, repeated.digest);
    assert_eq!(
        session_delivery_identity("thread", &one),
        session_delivery_identity("thread", &repeated)
    );
    assert_ne!(one.digest, two.digest);
    assert_ne!(
        session_delivery_identity("thread", &one),
        session_delivery_identity("thread", &two)
    );
}

#[test]
fn identical_commentary_in_different_turns_has_distinct_delivery_identity() {
    let collect = |turn: &str| {
        collect_items("thread",&[
        event(json!({"type":"event_msg","payload":{"type":"task_started","turn_id":turn}})),
        event(json!({"timestamp":"1","type":"event_msg","payload":{"type":"agent_message","message":"checking"}}))
    ],MirrorDetail::Send).remove(0)
    };
    assert_ne!(
        session_delivery_identity("thread", &collect("one")),
        session_delivery_identity("thread", &collect("two"))
    );
}

#[test]
fn equivalent_assistant_shapes_share_a_restart_stable_delivery_identity() {
    let items = collect_items(
        "thread-1",
        &[
            event(json!({
                "timestamp": "1",
                "type": "event_msg",
                "payload": {"type": "agent_message", "message": " same update "}
            })),
            event(json!({
                "timestamp": "2",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "phase": "commentary",
                    "content": [{"type": "output_text", "text": "same update"}]
                }
            })),
        ],
        MirrorDetail::Send,
    );

    assert_eq!(items.len(), 2);
    assert_ne!(items[0].digest, items[1].digest);
    let first = session_delivery_identity("thread-1", &items[0]);
    let second = session_delivery_identity("thread-1", &items[1]);
    assert_eq!(first, second);
    assert_eq!(first.domain(), SESSION_MIRROR_ASSISTANT_TEXT_NONCE_DOMAIN);
    assert_ne!(
        first,
        session_delivery_identity("thread-2", &items[0]),
        "thread identity must remain part of the logical key"
    );
}

#[test]
fn intentional_activity_duplicates_retain_distinct_event_identities() {
    let items = collect_items(
        "thread-1",
        &[event(json!({
            "timestamp": "2",
            "type": "response_item",
            "payload": {
                "type": "reasoning",
                "summary": [
                    {"type": "summary_text", "text": "checking"},
                    {"type": "summary_text", "text": "checking"}
                ]
            }
        }))],
        MirrorDetail::All,
    );

    assert_eq!(items.len(), 2);
    let first = session_delivery_identity("thread-1", &items[0]);
    let second = session_delivery_identity("thread-1", &items[1]);
    assert_eq!(first.domain(), SESSION_MIRROR_EVENT_NONCE_DOMAIN);
    assert_eq!(second.domain(), SESSION_MIRROR_EVENT_NONCE_DOMAIN);
    assert_ne!(first, second);
}
