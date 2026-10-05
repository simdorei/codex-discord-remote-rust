use serde_json::json;

#[test]
fn exact_publication_intent_is_non_control_but_never_native_approval() {
    for decision in ["ApproveExact", "KeepHeld"] {
        let payload = json!({"version":1,"work":{"Component":{"RecoveryPublicationDecision":{
            "proposal_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","revision":1,"decision":decision
        }}}});
        assert!(super::ordinary(&payload));
    }
}

#[test]
fn unknown_or_incomplete_publication_shape_remains_held() {
    let valid = json!({"version":1,"work":{"Component":{"RecoveryPublicationDecision":{
        "proposal_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","revision":1,"decision":"ApproveExact"
    }}}});
    for (field, bad) in [
        ("proposal_id", json!("AAA")),
        ("revision", json!(0)),
        ("revision", json!(1.0)),
        ("decision", json!("ApproveSession")),
        ("extra", json!(true)),
    ] {
        let mut payload = valid.clone();
        payload["work"]["Component"]["RecoveryPublicationDecision"][field] = bad;
        assert!(!super::ordinary(&payload));
    }
    let mut missing = valid;
    missing["work"]["Component"]["RecoveryPublicationDecision"]
        .as_object_mut()
        .unwrap()
        .remove("revision");
    assert!(!super::ordinary(&missing));
}
