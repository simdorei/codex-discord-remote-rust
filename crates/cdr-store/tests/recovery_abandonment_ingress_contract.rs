#[path = "support/recovery_abandonment_fixture.rs"]
mod fixture;

use cdr_store::async_resolution::abandonment as a;
use fixture::{Fixture, ID};
use serde_json::{Value, json};

fn native_payload(decision: &str) -> Value {
    json!({
        "version":1, "processing_mode":"normal",
        "work":{"Component":{"RecoveryAbandonDecision":{
            "proposal_id":ID, "revision":1, "decision":decision
        }}},
        "settings_binding":null, "request_rejection":null
    })
}

fn replace_payload(f: &Fixture, payload: &Value) {
    assert_eq!(
        f.db.execute(
            "UPDATE discord_ingress_journal SET payload_json=? WHERE ingress_id='interaction:90'",
            [payload.to_string()],
        )
        .unwrap(),
        1
    );
}

#[test]
fn actual_dispatcher_envelope_records_one_local_disposition() {
    let f = Fixture::new();
    assert_eq!(f.path.parent(), Some(f.temp.path()));
    f.delivered();
    f.click(a::Decision::AbandonOnly);
    replace_payload(&f, &native_payload("AbandonOnly"));
    let receipt = f.apply(a::Decision::AbandonOnly).unwrap();
    assert_eq!(receipt.decision, a::Decision::AbandonOnly);
    assert_eq!(f.count("codex_turn_queue"), 1);
    assert_eq!(f.count("codex_request_cancellations"), 1);
    assert_eq!(f.count("cdr_recovery_abandonment_decisions"), 1);
    assert!(cdr_store::async_resolution::admission_held(&f.path, fixture::TARGET).unwrap());
}

#[test]
fn actual_dispatcher_keep_held_never_deletes_or_cancels_the_original() {
    let f = Fixture::new();
    f.delivered();
    f.click(a::Decision::KeepHeld);
    replace_payload(&f, &native_payload("KeepHeld"));
    assert_eq!(
        f.apply(a::Decision::KeepHeld).unwrap().decision,
        a::Decision::KeepHeld
    );
    assert_eq!(f.count("codex_turn_queue"), 2);
    assert_eq!(f.count("codex_request_cancellations"), 0);
    assert_eq!(f.count("cdr_recovery_abandonment_decisions"), 1);
    assert!(cdr_store::async_resolution::admission_held(&f.path, fixture::TARGET).unwrap());
}

#[test]
fn historical_minimal_envelope_remains_supported() {
    let f = Fixture::new();
    f.delivered();
    f.click(a::Decision::AbandonOnly);
    assert!(f.apply(a::Decision::AbandonOnly).is_ok());
}

#[test]
fn actual_envelope_original_receipt_remains_readable_after_a_runtime_change() {
    let f = Fixture::new();
    f.delivered();
    f.click(a::Decision::AbandonOnly);
    replace_payload(&f, &native_payload("AbandonOnly"));
    let original = f.apply(a::Decision::AbandonOnly).unwrap();
    f.db.execute(
        "UPDATE codex_app_server_runtime SET runtime_id='cold-app'",
        [],
    )
    .unwrap();
    f.db.execute(
        "UPDATE codex_mutation_runtime SET runtime_id='cold-wire'",
        [],
    )
    .unwrap();
    assert_eq!(f.apply(a::Decision::AbandonOnly).unwrap(), original);
    assert_eq!(f.count("codex_request_cancellations"), 1);
    assert_eq!(f.count("cdr_recovery_abandonment_decisions"), 1);
}

#[test]
fn blocked_partial_unknown_and_conflicting_envelopes_never_authorize_disposition() {
    let base = native_payload("AbandonOnly");
    let mut cases = Vec::new();
    for (key, value) in [
        ("processing_mode", json!("sealed")),
        ("processing_mode", Value::Null),
        ("settings_binding", json!({"target":fixture::TARGET})),
        ("request_rejection", json!("rejected")),
        ("version", json!(true)),
        ("unexpected", json!(false)),
    ] {
        let mut value_case = base.clone();
        value_case[key] = value;
        cases.push(value_case);
    }
    for key in ["processing_mode", "settings_binding", "request_rejection"] {
        let mut value_case = base.clone();
        value_case.as_object_mut().unwrap().remove(key);
        cases.push(value_case);
    }
    let mut mixed = base.clone();
    mixed["work"]["Component"]["Approval"] =
        json!({"thread_id":fixture::TARGET,"answer":"Approve"});
    cases.push(mixed);
    let mut wrong_domain = base;
    wrong_domain["work"]["Component"] = json!({"RecoveryPublicationDecision":{
        "proposal_id":ID,"revision":1,"decision":"ApproveExact"
    }});
    cases.push(wrong_domain);
    for payload in cases {
        let f = Fixture::new();
        f.delivered();
        f.click(a::Decision::AbandonOnly);
        replace_payload(&f, &payload);
        assert!(f.apply(a::Decision::AbandonOnly).is_err(), "{payload}");
        f.unchanged();
    }
}
