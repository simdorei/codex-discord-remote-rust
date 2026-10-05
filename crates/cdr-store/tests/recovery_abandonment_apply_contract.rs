use cdr_store::{
    async_resolution::{self, abandonment as a},
    queue,
};
use serde_json::Value;
#[path = "support/recovery_abandonment_fixture.rs"]
mod fixture;
use fixture::{Fixture, ID, JOB, SIBLING, TARGET};

#[test]
fn exact_abandonment_is_atomic_audited_and_never_replayed() {
    let f = Fixture::new();
    let p = f.delivered();
    assert!(!p.review_text.contains("private exact input"));
    assert!(p.review_text.contains(JOB));
    assert!(p.review_text.contains("never replayed"));
    f.click(a::Decision::AbandonOnly);
    let receipt = f.apply(a::Decision::AbandonOnly).unwrap();
    assert_eq!(receipt.job_id, JOB);
    assert_eq!(receipt.decision, a::Decision::AbandonOnly);
    assert_eq!(f.count("codex_turn_queue"), 1);
    assert_eq!(f.count("codex_request_cancellations"), 1);
    assert_eq!(f.count("cdr_recovery_abandonment_decisions"), 1);
    let retained: String =
        f.db.query_row("SELECT job_id FROM codex_turn_queue", [], |r| r.get(0))
            .unwrap();
    assert_eq!(retained, SIBLING);
    let sealed: String =
        f.db.query_row(
            "SELECT seal_json FROM cdr_recovery_abandonment_proposals",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let _: Value = serde_json::from_str(&sealed).unwrap();
    for original in [
        "private exact input",
        "Keep CASE",
        "356",
        "original held evidence",
    ] {
        assert!(sealed.contains(original));
    }
    assert!(async_resolution::admission_held(&f.path, TARGET).unwrap());
    assert!(
        queue::try_begin_attempt(&f.path, JOB, &[], 1)
            .unwrap()
            .is_none()
    );
    assert!(
        f.db.execute("DELETE FROM codex_request_cancellations", [])
            .is_err()
    );
    assert_eq!(f.apply(a::Decision::AbandonOnly).unwrap(), receipt);
    assert_eq!(a::decision_status(&f.path, ID, 1).unwrap(), Some(receipt));
}

#[test]
fn keep_held_is_a_distinct_immutable_decision() {
    let f = Fixture::new();
    f.delivered();
    f.click(a::Decision::KeepHeld);
    let first = f.apply(a::Decision::KeepHeld).unwrap();
    assert_eq!(f.apply(a::Decision::KeepHeld).unwrap(), first);
    assert!(f.apply(a::Decision::AbandonOnly).is_err());
    assert_eq!(f.count("codex_turn_queue"), 2);
    assert_eq!(f.count("codex_request_cancellations"), 0);
}

#[test]
fn cold_duplicate_reads_exact_receipt_without_new_authority() {
    let f = Fixture::new();
    f.delivered();
    f.click(a::Decision::AbandonOnly);
    let first = f.apply(a::Decision::AbandonOnly).unwrap();
    f.db.execute(
        "UPDATE codex_app_server_runtime SET runtime_id='next-app'",
        [],
    )
    .unwrap();
    f.db.execute(
        "UPDATE discord_ingress_journal SET state='completed',phase='confirmed'
        WHERE ingress_id='interaction:90'",
        [],
    )
    .unwrap();
    let receipt = a::record_decision(
        &f.path,
        &a::DecisionInput {
            proposal_id: ID,
            revision: 1,
            ingress_id: "interaction:90",
            decision: a::Decision::AbandonOnly,
            now: 1000.0,
        },
    )
    .unwrap();
    assert_eq!(receipt, first);
    assert_eq!(f.count("codex_request_cancellations"), 1);
}

#[test]
fn snapshot_or_owner_change_rejects_before_disposition() {
    for change in [
        "UPDATE codex_turn_queue SET prompt='changed' WHERE job_id='b3d5a1a3-5c3e-4764-967b-0cef767efde9'",
        "UPDATE codex_turn_queue SET baseline_turn_ids='[\"new\"]' WHERE job_id='b3d5a1a3-5c3e-4764-967b-0cef767efde9'",
        "UPDATE codex_turn_queue SET turn_id='already-owned' WHERE job_id='b3d5a1a3-5c3e-4764-967b-0cef767efde9'",
        "UPDATE codex_turn_queue SET prompt='sibling changed' WHERE job_id='bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'",
        "UPDATE mirror_threads SET discord_thread_id=21",
        "UPDATE codex_app_server_runtime SET runtime_id='new-app'",
        "UPDATE codex_mutation_runtime SET runtime_id='new-wire'",
    ] {
        let f = Fixture::new();
        f.delivered();
        f.click(a::Decision::AbandonOnly);
        f.db.execute_batch(change).unwrap();
        assert!(f.apply(a::Decision::AbandonOnly).is_err(), "{change}");
        f.unchanged();
    }
}

#[test]
fn authenticated_saved_click_headers_and_component_are_required() {
    for change in [
        "UPDATE discord_ingress_journal SET owner_user_id=31 WHERE kind='interaction'",
        "UPDATE discord_ingress_journal SET application_id=51 WHERE kind='interaction'",
        "UPDATE discord_ingress_journal SET channel_id=21 WHERE kind='interaction'",
        "UPDATE discord_ingress_journal SET source_message_id=61 WHERE kind='interaction'",
        "UPDATE discord_ingress_journal SET target_thread_id='other' WHERE kind='interaction'",
        "UPDATE discord_ingress_journal SET runtime_id='old' WHERE kind='interaction'",
        "UPDATE discord_ingress_journal SET state='held' WHERE kind='interaction'",
        "UPDATE discord_ingress_journal SET owner_kind='queue',owner_id='other' WHERE kind='interaction'",
        "UPDATE discord_ingress_journal SET payload_json=replace(payload_json,'RecoveryAbandonDecision','RecoveryPublicationDecision') WHERE kind='interaction'",
        "UPDATE discord_ingress_journal SET payload_json=replace(payload_json,'AbandonOnly','KeepHeld') WHERE kind='interaction'",
    ] {
        let f = Fixture::new();
        f.delivered();
        f.click(a::Decision::AbandonOnly);
        f.db.execute_batch(change).unwrap();
        assert!(f.apply(a::Decision::AbandonOnly).is_err(), "{change}");
        f.unchanged();
    }
}

#[test]
fn source_message_must_be_exact_owned_authenticated_proposal_request() {
    for change in [
        "UPDATE discord_ingress_journal SET owner_user_id=31",
        "UPDATE discord_ingress_journal SET target_thread_id='other'",
        "UPDATE discord_ingress_journal SET source_message_id=81",
        "UPDATE discord_ingress_journal SET payload_json=replace(payload_json,'DiscardRequest','SavedRequest')",
        "UPDATE discord_ingress_journal SET state='completed'",
        "UPDATE discord_ingress_journal SET runtime_id='old'",
        "UPDATE codex_turn_queue SET owner_user_id=NULL",
    ] {
        let f = Fixture::new();
        f.db.execute_batch(change).unwrap();
        assert!(
            a::propose(
                &f.path,
                &a::ProposalInput {
                    proposal_id: ID,
                    job_id: JOB,
                    ingress_id: "message:80",
                    application_id: 50,
                    now: 10.0,
                    expires_at: 100.0,
                }
            )
            .is_err(),
            "{change}"
        );
        f.unchanged();
    }
}

#[test]
fn database_faults_leave_no_partial_disposition() {
    for trigger in [
        "CREATE TRIGGER fail_decision BEFORE INSERT ON cdr_recovery_abandonment_decisions BEGIN SELECT RAISE(ABORT,'injected'); END",
        "CREATE TRIGGER fail_decision BEFORE INSERT ON cdr_recovery_abandonment_decisions BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER fail_cancel BEFORE INSERT ON codex_request_cancellations BEGIN SELECT RAISE(ABORT,'injected'); END",
        "CREATE TRIGGER fail_cancel BEFORE INSERT ON codex_request_cancellations BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER fail_delete BEFORE DELETE ON codex_turn_queue BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER alter_sibling AFTER DELETE ON codex_turn_queue BEGIN UPDATE codex_turn_queue SET prompt='altered'; END",
        "CREATE TRIGGER alter_original AFTER INSERT ON cdr_recovery_abandonment_decisions BEGIN UPDATE codex_turn_queue SET prompt='altered'; END",
    ] {
        let f = Fixture::new();
        f.delivered();
        f.click(a::Decision::AbandonOnly);
        f.db.execute_batch(trigger).unwrap();
        assert!(f.apply(a::Decision::AbandonOnly).is_err(), "{trigger}");
        f.unchanged();
        let input: String =
            f.db.query_row(
                "SELECT prompt FROM codex_turn_queue WHERE job_id=?",
                [JOB],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(input, "private exact input\nKeep CASE");
    }
}

#[test]
fn expiry_revision_delivery_and_other_database_cannot_authorize() {
    let f = Fixture::new();
    let p = f.propose();
    f.click(a::Decision::AbandonOnly);
    assert!(f.apply(a::Decision::AbandonOnly).is_err());
    assert!(a::bind_delivery(&f.path, ID, 60, "wrong", 11.0).is_err());
    a::bind_delivery(&f.path, ID, 60, &p.review_sha256, 11.0).unwrap();
    assert!(a::bind_delivery(&f.path, ID, 61, &p.review_sha256, 11.0).is_err());
    for (revision, now) in [(2, 13.0), (0, 13.0), (1, 100.0), (1, f64::NAN)] {
        assert!(
            a::record_decision(
                &f.path,
                &a::DecisionInput {
                    proposal_id: ID,
                    revision,
                    ingress_id: "interaction:90",
                    decision: a::Decision::AbandonOnly,
                    now,
                }
            )
            .is_err()
        );
    }
    let copy = f.temp.path().join("different-install.sqlite");
    f.db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    std::fs::copy(&f.path, &copy).unwrap();
    assert!(
        a::record_decision(
            &copy,
            &a::DecisionInput {
                proposal_id: ID,
                revision: 1,
                ingress_id: "interaction:90",
                decision: a::Decision::AbandonOnly,
                now: 13.0,
            }
        )
        .is_err()
    );
    f.unchanged();
}

#[test]
fn readonly_delivery_is_not_a_current_application_grant() {
    let f = Fixture::new();
    let p = f.delivered();
    let before = std::fs::read(&f.path).unwrap();
    let delivery = a::delivered_proposal(&f.path, ID, 1).unwrap();
    assert_eq!(delivery.proposal, p);
    delivery.require_actor(50, 20, 30, 60).unwrap();
    for (app, channel, actor, message) in [
        (51, 20, 30, 60),
        (50, 21, 30, 60),
        (50, 20, 31, 60),
        (50, 20, 30, 61),
    ] {
        assert!(
            delivery
                .require_actor(app, channel, actor, message)
                .is_err()
        );
    }
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
    assert!(a::delivered_proposal(&f.path, ID, 2).is_err());
    let missing = f.temp.path().join("absent.sqlite");
    assert!(a::decision_status(&missing, ID, 1).is_err());
    assert!(!missing.exists());
}
#[test]
fn original_event_and_direct_writer_remain_revoked_after_abandonment() {
    let f = Fixture::new();
    f.delivered();
    f.click(a::Decision::AbandonOnly);
    f.apply(a::Decision::AbandonOnly).unwrap();
    for job in [JOB, "dddddddd-dddd-4ddd-8ddd-dddddddddddd"] {
        assert!(
            queue::enqueue(
                &f.path,
                queue::NewQueueJob {
                    job_id: job,
                    target_thread_id: TARGET,
                    channel_id: 20,
                    owner_user_id: Some(30),
                    discord_message_id: Some(70),
                    app_server_generation: 2,
                    prompt: "private exact input\nKeep CASE",
                    queued: true,
                    ack_sent: false,
                    created_at: 20.0,
                }
            )
            .is_err()
        );
    }
    assert!(
        f.db.execute(
            "UPDATE codex_turn_queue SET job_id=? WHERE job_id=?",
            [JOB, SIBLING]
        )
        .is_err()
    );
    assert!(
        queue::start_authority::validate_in(&f.db, &serde_json::json!({"job_id":JOB}), TARGET, 2)
            .is_err()
    );
    assert_eq!(f.count("codex_turn_queue"), 1);
    assert_eq!(f.count("codex_request_cancellations"), 1);
}

#[test]
fn another_click_cannot_reconsume_a_committed_decision() {
    let f = Fixture::new();
    f.delivered();
    f.click(a::Decision::AbandonOnly);
    let first = f.apply(a::Decision::AbandonOnly).unwrap();
    f.db.execute(
        "INSERT INTO discord_ingress_journal
        (ingress_id,kind,event_id,application_id,channel_id,owner_user_id,source_message_id,
         payload_json,runtime_id,state,phase,target_thread_id,created_at,updated_at)
        SELECT 'interaction:91',kind,91,application_id,channel_id,owner_user_id,source_message_id,
         payload_json,runtime_id,state,phase,target_thread_id,14,14
        FROM discord_ingress_journal WHERE ingress_id='interaction:90'",
        [],
    )
    .unwrap();
    assert!(
        a::record_decision(
            &f.path,
            &a::DecisionInput {
                proposal_id: ID,
                revision: 1,
                ingress_id: "interaction:91",
                decision: a::Decision::AbandonOnly,
                now: 14.0,
            }
        )
        .is_err()
    );
    assert_eq!(a::decision_status(&f.path, ID, 1).unwrap(), Some(first));
    assert_eq!(f.count("cdr_recovery_abandonment_decisions"), 1);
}

#[test]
fn new_revision_supersedes_but_never_rewrites_an_old_proposal() {
    let f = Fixture::new();
    let first = f.delivered();
    assert_eq!(f.propose(), first);
    f.click(a::Decision::AbandonOnly);
    let next = a::propose(
        &f.path,
        &a::ProposalInput {
            proposal_id: "dddddddddddddddddddddddddddddddd",
            job_id: JOB,
            ingress_id: "message:80",
            application_id: 50,
            now: 12.0,
            expires_at: 100.0,
        },
    )
    .unwrap();
    assert_eq!(next.revision, 2);
    assert!(f.apply(a::Decision::AbandonOnly).is_err());
    assert_eq!(
        a::delivered_proposal(&f.path, ID, 1).unwrap().proposal,
        first
    );
    assert_eq!(f.count("cdr_recovery_abandonment_proposals"), 2);
    f.unchanged();
}

#[test]
fn abandon_only_preserves_existing_archive_stop_and_execution_holds() {
    for phase in ["attempted", "verified"] {
        let f = Fixture::new();
        // The store fixture supplies saved custody. Real admission exemptions
        // and shared-lock ordering belong to the separate runtime route gate.
        f.click(a::Decision::AbandonOnly);
        f.db.execute(
            "INSERT INTO codex_archive_fences VALUES(?,'archive-op',NULL,?)",
            rusqlite::params![TARGET, phase],
        )
        .unwrap();
        f.db.execute(
            "INSERT INTO cdr_stop_controls
            (operation_id,target_thread_id,resident_owner,generation,turn_id,record_json,phase)
            VALUES('stop-op',?,'original-resident',1,'original-turn','{}','unknown')",
            [TARGET],
        )
        .unwrap();
        f.db.execute(
            "INSERT INTO cdr_execution_holds VALUES(?,?,'original unknown','{}',1)",
            [JOB, TARGET],
        )
        .unwrap();
        f.delivered();
        f.apply(a::Decision::AbandonOnly).unwrap();
        let retained: String =
            f.db.query_row("SELECT phase FROM codex_archive_fences", [], |r| r.get(0))
                .unwrap();
        assert_eq!(retained, phase);
        let retained: String =
            f.db.query_row("SELECT phase FROM cdr_stop_controls", [], |r| r.get(0))
                .unwrap();
        assert_eq!(retained, "unknown");
        assert_eq!(f.count("cdr_execution_holds"), 1);
        assert!(async_resolution::admission_held(&f.path, TARGET).unwrap());
    }
}

#[test]
fn a_new_barrier_or_prepared_writer_invalidates_the_snapshot() {
    for change in [
        "INSERT INTO codex_archive_fences VALUES('01a06156-56cd-70b0-af02-2de7445ba4c7','later-archive',NULL,'attempted')",
        "INSERT INTO cdr_stop_controls(operation_id,target_thread_id,resident_owner,generation,turn_id,record_json,phase)
         VALUES('later-stop','01a06156-56cd-70b0-af02-2de7445ba4c7','resident',1,'turn','{}','unknown')",
        "INSERT INTO codex_mutation_attempts(attempt_id,runtime_id,owner_id,generation,wire_id,method,target_thread_id,
         scoped,request_sha256,state,created_at,updated_at)
         VALUES('wire','wire-fixture','owner',1,'w','turn/start','01a06156-56cd-70b0-af02-2de7445ba4c7',1,'hash','prepared',12,12)",
    ] {
        let f = Fixture::new();
        f.delivered();
        f.click(a::Decision::AbandonOnly);
        f.db.execute_batch(change).unwrap();
        assert!(f.apply(a::Decision::AbandonOnly).is_err(), "{change}");
        f.unchanged();
    }
}

#[test]
fn unsupported_schema_is_not_repaired_by_apply() {
    let f = Fixture::new();
    f.delivered();
    f.click(a::Decision::AbandonOnly);
    f.db.execute_batch("DROP TRIGGER cdr_recovery_abandonment_proposal_no_delete")
        .unwrap();
    let before = std::fs::read(&f.path).unwrap();
    assert!(f.apply(a::Decision::AbandonOnly).is_err());
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
    let missing: i64 =
        f.db.query_row(
            "SELECT count(*) FROM sqlite_schema
        WHERE name='cdr_recovery_abandonment_proposal_no_delete'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(missing, 0);
    assert_eq!(f.count("codex_turn_queue"), 2);
}

#[test]
fn two_database_connections_observe_one_durable_disposition() {
    let f = Fixture::new();
    f.delivered();
    f.click(a::Decision::AbandonOnly);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut joins = Vec::new();
    for _ in 0..2 {
        let path = f.path.clone();
        let barrier = barrier.clone();
        joins.push(std::thread::spawn(move || {
            barrier.wait();
            a::record_decision(
                &path,
                &a::DecisionInput {
                    proposal_id: ID,
                    revision: 1,
                    ingress_id: "interaction:90",
                    decision: a::Decision::AbandonOnly,
                    now: 13.0,
                },
            )
            .unwrap()
        }));
    }
    barrier.wait();
    let receipts: Vec<_> = joins.into_iter().map(|join| join.join().unwrap()).collect();
    assert_eq!(receipts[0], receipts[1]);
    assert_eq!(f.count("codex_request_cancellations"), 1);
    assert_eq!(f.count("cdr_recovery_abandonment_decisions"), 1);
}

#[test]
fn commit_busy_rolls_back_and_retry_reconciles_by_exact_decision_identity() {
    let f = Fixture::new();
    f.delivered();
    f.click(a::Decision::AbandonOnly);
    let mode: String =
        f.db.query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))
            .unwrap();
    assert_eq!(mode, "delete");
    let reader = rusqlite::Connection::open(&f.path).unwrap();
    reader.execute_batch("BEGIN DEFERRED").unwrap();
    let _: i64 = reader
        .query_row("SELECT count(*) FROM codex_turn_queue", [], |r| r.get(0))
        .unwrap();
    assert!(f.apply(a::Decision::AbandonOnly).is_err());
    reader.execute_batch("ROLLBACK").unwrap();
    assert_eq!(f.count("codex_turn_queue"), 2);
    assert_eq!(f.count("codex_request_cancellations"), 0);
    assert_eq!(f.count("cdr_recovery_abandonment_decisions"), 0);
    let receipt = f.apply(a::Decision::AbandonOnly).unwrap();
    assert_eq!(a::decision_status(&f.path, ID, 1).unwrap(), Some(receipt));
}

#[test]
fn unstarted_identity_and_bounded_private_snapshot_are_required() {
    for change in [
        "UPDATE codex_turn_queue SET goal_waiting=1",
        "UPDATE codex_turn_queue SET state='running',turn_id='real-owned-turn'",
        "UPDATE codex_turn_queue SET discord_message_id=NULL",
        "UPDATE codex_turn_queue SET prompt=replace(hex(zeroblob(70000)),'0','p')",
    ] {
        let f = Fixture::new();
        f.db.execute_batch(change).unwrap();
        assert!(
            a::propose(
                &f.path,
                &a::ProposalInput {
                    proposal_id: ID,
                    job_id: JOB,
                    ingress_id: "message:80",
                    application_id: 50,
                    now: 10.0,
                    expires_at: 100.0,
                }
            )
            .is_err(),
            "{change}"
        );
        assert_eq!(f.count("cdr_recovery_abandonment_proposals"), 0);
        assert_eq!(f.count("codex_request_cancellations"), 0);
    }
}
