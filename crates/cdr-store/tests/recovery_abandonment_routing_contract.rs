#[allow(dead_code)]
#[path = "support/recovery_abandonment_fixture.rs"]
mod fixture;
use cdr_store::async_resolution::abandonment as store;
use fixture::{Fixture, ID, JOB, TARGET};

fn route(id: &str, revision: i64) -> store::DecisionRouteInput<'_> {
    store::DecisionRouteInput {
        proposal_id: id,
        revision,
        interaction_id: 90,
        application_id: 50,
        channel_id: 20,
        owner_user_id: 30,
        source_message_id: 60,
        decision: store::Decision::AbandonOnly,
        now: 13.0,
    }
}

#[test]
fn command_route_requires_exact_original_owner_channel_and_held_pending() {
    let f = Fixture::new();
    assert_eq!(store::command_target(&f.path, JOB, 20, 30).unwrap(), TARGET);
    for (job, channel, owner) in [(JOB, 20, 31), (JOB, 21, 30), ("not-a-uuid", 20, 30)] {
        assert!(store::command_target(&f.path, job, channel, owner).is_err());
    }
    assert!(store::command_target(&f.path, &JOB.to_uppercase(), 20, 30).is_err());
    f.db.execute(
        "UPDATE codex_turn_queue SET goal_waiting=1 WHERE job_id=?",
        [JOB],
    )
    .unwrap();
    assert!(store::command_target(&f.path, JOB, 20, 30).is_err());
    assert!(store::command_target(&f.temp.path().join("absent.sqlite"), JOB, 20, 30).is_err());
    assert!(!f.temp.path().join("absent.sqlite").exists());
}

#[test]
fn pre_ack_route_checks_identity_expiry_and_latest_revision_without_ingress_writes() {
    let f = Fixture::new();
    let p = f.delivered();
    let before = f.count("discord_ingress_journal");
    assert_eq!(
        store::authorize_decision(&f.path, &route(ID, 1))
            .unwrap()
            .proposal,
        p
    );
    for case in 0..8 {
        let mut bad = route(ID, 1);
        match case {
            0 => bad.application_id = 51,
            1 => bad.channel_id = 21,
            2 => bad.owner_user_id = 31,
            3 => bad.source_message_id = 61,
            4 => bad.revision = 2,
            5 => bad.now = 100.0,
            6 => bad.interaction_id = 0,
            _ => bad.now = f64::NAN,
        }
        assert!(store::authorize_decision(&f.path, &bad).is_err());
    }
    store::propose(
        &f.path,
        &store::ProposalInput {
            proposal_id: "dddddddddddddddddddddddddddddddd",
            job_id: JOB,
            ingress_id: "message:80",
            application_id: 50,
            now: 12.0,
            expires_at: 100.0,
        },
    )
    .unwrap();
    assert!(store::authorize_decision(&f.path, &route(ID, 1)).is_err());
    assert_eq!(f.count("discord_ingress_journal"), before);
    f.unchanged();
}

#[test]
fn committed_history_is_only_for_the_same_original_event_and_choice() {
    for choice in [store::Decision::AbandonOnly, store::Decision::KeepHeld] {
        let f = Fixture::new();
        f.delivered();
        f.click(choice);
        let receipt = f.apply(choice).unwrap();
        let mut input = route(ID, 1);
        input.decision = choice;
        input.now = 200.0;
        assert!(store::authorize_decision(&f.path, &input).is_ok());
        input.interaction_id = 91;
        assert!(store::authorize_decision(&f.path, &input).is_err());
        input.interaction_id = 90;
        input.decision = if choice == store::Decision::KeepHeld {
            store::Decision::AbandonOnly
        } else {
            store::Decision::KeepHeld
        };
        assert!(store::authorize_decision(&f.path, &input).is_err());
        assert_eq!(
            store::decision_status(&f.path, ID, 1).unwrap(),
            Some(receipt)
        );
    }
}

#[test]
fn changed_snapshot_database_or_missing_runtime_fail_closed_before_ack() {
    for sql in [
        "UPDATE codex_turn_queue SET prompt='changed private evidence'",
        "UPDATE mirror_threads SET discord_thread_id=21",
        "UPDATE codex_app_server_runtime SET runtime_id='different-runtime'",
        "DROP TABLE codex_mutation_runtime",
    ] {
        let f = Fixture::new();
        f.delivered();
        let copied = f.temp.path().join("copied.sqlite");
        std::fs::copy(&f.path, &copied).unwrap();
        assert!(store::authorize_decision(&copied, &route(ID, 1)).is_err());
        f.db.execute_batch(sql).unwrap();
        assert!(store::authorize_decision(&f.path, &route(ID, 1)).is_err());
        assert_eq!(f.count("cdr_recovery_abandonment_decisions"), 0);
        assert_eq!(f.count("codex_turn_queue"), 2);
    }
}
