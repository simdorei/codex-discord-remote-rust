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

#[test]
fn recovered_result_allows_retry_only_with_exact_no_dispatch_proof() {
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
        (json!({}), false),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("store.sqlite");
        let choice = setup(&db, &outcome);
        assert_eq!(
            ingress::recover_prior_runtime(&db, "new-runtime", 5.5).unwrap(),
            1
        );
        let original = ingress::get(&db, "interaction:1").unwrap().unwrap();
        assert_eq!(
            (&*original.state, &*original.phase),
            ("held", "result_recorded")
        );
        assert!(original.owner_id.is_none());
        let retry =
            ingress::admit_busy_interaction(&db, &request(2, 6.0), &choice, "queue").unwrap();
        assert_eq!(retry.created, allowed, "{outcome}");
        assert_eq!(retry.canonical_repeat_created, !allowed);
        assert!(
            !ingress::admit_busy_interaction(&db, &request(1, 7.0), &choice, "steer")
                .unwrap()
                .created
        );
        assert!(
            ingress::admit_busy_interaction(&db, &request(3, 8.0), &choice, "queue")
                .unwrap()
                .canonical_repeat_created
        );
    }
}

#[test]
fn rejected_control_stays_outside_queue_confirmation_before_and_after_restart() {
    for restarted in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("store.sqlite");
        let outcome = json!({"kind":"busy_control_preflight_rejected","control_dispatched":false});
        let choice_id = setup(&db, &outcome);
        if restarted {
            ingress::recover_prior_runtime(&db, "new-runtime", 5.5).unwrap();
        }
        let original = ingress::get(&db, "interaction:1").unwrap().unwrap();
        let choice = cdr_store::claims::get_busy_choice(&db, &choice_id, 6.0)
            .unwrap()
            .unwrap();
        assert!(
            ingress::admit_busy_interaction(&db, &request(2, 6.0), &choice_id, "queue")
                .unwrap()
                .created
        );
        let accepted = cdr_store::prompt_intake::admit_busy_queue(
            &db, &choice, "original", false, "accepted", 7.0,
        )
        .unwrap();
        assert!(accepted.intake.is_some());
        let rejection = ingress::get(&db, "interaction:1").unwrap().unwrap();
        assert_eq!(
            rejection.owner_id, None,
            "rejected control became a final-delivery barrier"
        );
        assert_eq!(rejection.state, original.state);
        assert_eq!(rejection.phase, "result_recorded");
        assert_eq!(rejection.outcome, Some(outcome));
        assert!(!rejection.confirmation_delivered);
        let pending = || {
            cdr_store::schema::open_initialized(&db).unwrap().query_row(
            "SELECT COUNT(*) FROM discord_ingress_journal WHERE owner_id=? AND confirmation_delivered=0",
            [&accepted.job_id], |row| row.get::<_, i64>(0),
        ).unwrap()
        };
        assert_eq!(
            pending(),
            1,
            "accepted queue confirmation must still block delivery"
        );
        ingress::confirm(&db, "interaction:2", 8.0).unwrap();
        assert_eq!(pending(), 0, "confirmed acceptance must release the final");
        assert!(
            cdr_store::prompt_intake::admit_busy_queue(
                &db, &choice, "original", false, "accepted", 9.0
            )
            .unwrap()
            .intake
            .is_none()
        );
        assert_eq!(
            ingress::get(&db, "interaction:1")
                .unwrap()
                .unwrap()
                .owner_id,
            None
        );
        assert_eq!(pending(), 0);
    }
}

#[test]
fn queue_ownership_retains_unconfirmed_uncertain_or_dispatched_controls() {
    for restarted in [false, true] {
        for outcome in [
            json!({"kind":"busy_control_preflight_rejected","control_dispatched":true}),
            json!({"kind":"busy_control_preflight_rejected","control_dispatched":"false"}),
            json!({"kind":"busy_control_preflight_rejected","control_dispatched":0}),
            json!({"kind":"busy_control_preflight_rejected"}),
            json!({"kind":"other","control_dispatched":false}),
            json!({}),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let db = temp.path().join("store.sqlite");
            let choice_id = setup(&db, &outcome);
            if restarted {
                ingress::recover_prior_runtime(&db, "new-runtime", 5.5).unwrap();
            }
            let choice = cdr_store::claims::get_busy_choice(&db, &choice_id, 6.0)
                .unwrap()
                .unwrap();
            let accepted = cdr_store::prompt_intake::admit_busy_queue(
                &db, &choice, "original", false, "accepted", 7.0,
            )
            .unwrap();
            let original = ingress::get(&db, "interaction:1").unwrap().unwrap();
            assert_eq!(
                original.owner_id.as_deref(),
                Some(accepted.job_id.as_str()),
                "{outcome}"
            );
            assert!(!original.confirmation_delivered);
            assert_eq!(original.outcome, Some(outcome));
        }
    }
}
