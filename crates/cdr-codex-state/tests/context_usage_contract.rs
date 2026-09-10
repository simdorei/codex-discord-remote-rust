use cdr_codex_state::context_usage_from_events;
use serde_json::{Value, json};

fn usage(input: Value, total: Value) -> Value {
    let mut event = json!({"type":"event_msg","timestamp":"2026-09-07T10:00:00Z","payload":{"type":"token_count",
        "info":{"last_token_usage":{},
        "total_token_usage":{"input_tokens":90_000_000,"total_tokens":100_000_000}}}});
    event["payload"]["info"]["last_token_usage"]["input_tokens"] = input;
    event["payload"]["info"]["last_token_usage"]["total_tokens"] = total;
    event
}

#[test]
fn current_and_peak_input_are_distinct_from_cumulative_usage_and_inferred_compaction() {
    let events = vec![
        json!({"type":"event_msg","payload":{"type":"task_started","model_context_window":200_000}}),
        usage(json!(150_000), json!(151_000)),
        usage(json!(60_000), json!(61_000)),
        usage(json!(80_000), json!(82_000)),
    ];
    let result = context_usage_from_events(&events)
        .unwrap()
        .expect("token observation");
    assert_eq!(result.last_input_tokens, 80_000);
    assert_eq!(result.peak_input_tokens, 150_000);
    assert_eq!(result.last_total_tokens, Some(82_000));
    assert_eq!(result.model_context_window, Some(200_000));
    assert_eq!(result.inferred_compactions, 1);
    assert_eq!(result.last_compaction, Some((150_000, 60_000)));
    assert_eq!(result.observed_at.as_deref(), Some("2026-09-07T10:00:00Z"));
}

#[test]
fn unknown_window_and_missing_total_are_not_invented_as_zero() {
    let mut event = usage(json!(42), Value::Null);
    event["payload"]["info"]["last_token_usage"]
        .as_object_mut()
        .unwrap()
        .remove("total_tokens");
    let result = context_usage_from_events([&event])
        .unwrap()
        .expect("input observed");
    assert_eq!(result.model_context_window, None);
    assert_eq!(result.last_total_tokens, None);
    assert_eq!(result.last_input_tokens, 42);
    assert_eq!(
        context_usage_from_events([
            &json!({"type":"event_msg","payload":{"type":"token_count","info":null}})
        ])
        .unwrap(),
        None
    );
}

#[test]
fn malformed_latest_input_does_not_silently_reuse_an_older_value() {
    for bad in [
        json!(true),
        json!(-1),
        json!("bad"),
        json!(1.5),
        Value::Null,
    ] {
        let events = [usage(json!(100), json!(120)), usage(bad, json!(130))];
        assert!(context_usage_from_events(&events).is_err());
    }
    let mut event = usage(json!(123), json!(124));
    event["type"] = json!("unrelated");
    assert_eq!(context_usage_from_events([&event]).unwrap(), None);
}

#[test]
fn changing_window_is_not_reported_as_compaction_or_retroactively_applied() {
    let events = vec![
        json!({"type":"event_msg","payload":{"type":"task_started","model_context_window":200_000}}),
        usage(json!(150_000), json!(151_000)),
        json!({"type":"event_msg","payload":{"type":"task_started","model_context_window":100_000}}),
    ];
    let old = context_usage_from_events(&events).unwrap().unwrap();
    assert_eq!(old.model_context_window, Some(200_000));
    let next = usage(json!(50_000), json!(51_000));
    let updated = context_usage_from_events(events.iter().chain([&next]))
        .unwrap()
        .unwrap();
    assert_eq!(updated.model_context_window, Some(100_000));
    assert_eq!(updated.inferred_compactions, 0);
}

#[test]
fn zero_is_a_real_measurement_but_does_not_count_as_inferred_compaction() {
    let events = [
        usage(json!(150_000), json!(151_000)),
        usage(json!(0), json!(0)),
    ];
    let result = context_usage_from_events(&events).unwrap().unwrap();
    assert_eq!(result.last_input_tokens, 0);
    assert_eq!(result.last_total_tokens, Some(0));
    assert_eq!(result.peak_input_tokens, 150_000);
    assert_eq!(result.inferred_compactions, 0);
}
