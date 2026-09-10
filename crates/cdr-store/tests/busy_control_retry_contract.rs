use cdr_store::{
    claims::{NewBusyChoice, create_busy_choice},
    ingress::{self, IngressKind, NewIngress},
};
use serde_json::{Value, json};

fn request(id: i64, now: f64) -> NewIngress {
    NewIngress {
        ingress_id: format!("interaction:{id}"),
        kind: IngressKind::Interaction,
        event_id: Some(id),
        application_id: Some(1),
        channel_id: 42,
        owner_user_id: 3,
        source_message_id: Some(100),
        payload: json!({"version":1}),
        target_thread_id: None,
        canonical_owner: None,
        now,
    }
}

fn setup(db: &std::path::Path, outcome: &Value) -> String {
    let choice = create_busy_choice(
        db,
        NewBusyChoice {
            owner_user_id: 3,
            channel_id: 42,
            target_thread_id: Some("original"),
            prompt: "extra direction",
            allow_steer: false,
            now: 1.0,
            time_to_live: 100.0,
        },
    )
    .unwrap();
    assert!(
        ingress::admit_busy_interaction(db, &request(1, 2.0), &choice, "steer")
            .unwrap()
            .created
    );
    assert!(ingress::acknowledge(db, "interaction:1", 3.0).unwrap());
    assert!(ingress::begin_execution(db, "interaction:1", "processing", None, 4.0).unwrap());
    ingress::record_result(db, "interaction:1", outcome, 5.0).unwrap();
    choice
}

#[test]
fn only_exact_durable_no_dispatch_proof_allows_a_new_execution() {
    for (outcome, allowed) in [
        (
            json!({"kind":"busy_control_preflight_rejected","control_dispatched":false}),
            true,
        ),
        (
            json!({"kind":"busy_control_preflight_rejected","control_dispatched":true}),
            false,
        ),
        (
            json!({"kind":"busy_control_preflight_rejected","control_dispatched":"false"}),
            false,
        ),
        (
            json!({"kind":"busy_control_preflight_rejected","control_dispatched":0}),
            false,
        ),
        (json!({"kind":"busy_control_preflight_rejected"}), false),
        (json!({"kind":"other","control_dispatched":false}), false),
        (json!({"action_completed":true}), false),
        (json!({}), false),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("store.sqlite");
        let choice = setup(&db, &outcome);
        let retry =
            ingress::admit_busy_interaction(&db, &request(2, 6.0), &choice, "steer").unwrap();
        assert_eq!(retry.created, allowed, "{outcome}");
        assert_eq!(retry.canonical_repeat_created, !allowed);
        // A new execution in flight blocks any following click regardless of
        // the safe rejection older in the chain. The old event never reexecutes.
        assert!(
            !ingress::admit_busy_interaction(&db, &request(1, 7.0), &choice, "steer")
                .unwrap()
                .created
        );
        assert!(
            ingress::admit_busy_interaction(&db, &request(3, 8.0), &choice, "steer")
                .unwrap()
                .canonical_repeat_created
        );
    }
}

#[test]
fn safe_retry_does_not_bypass_expiration_or_original_user_and_channel() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("store.sqlite");
    let choice = setup(
        &db,
        &json!({"kind":"busy_control_preflight_rejected","control_dispatched":false}),
    );
    let mut foreign = request(2, 6.0);
    foreign.owner_user_id = 4;
    assert!(ingress::admit_busy_interaction(&db, &foreign, &choice, "steer").is_err());
    foreign.owner_user_id = 3;
    foreign.channel_id = 43;
    assert!(ingress::admit_busy_interaction(&db, &foreign, &choice, "steer").is_err());
    assert!(ingress::admit_busy_interaction(&db, &request(2, 102.0), &choice, "steer").is_err());
}
