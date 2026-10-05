//! A positive prerequisite report is deliberately not permission to execute.
use cdr_store::{
    async_question as aq,
    async_resolution::abandonment::{self as a, readiness as r},
    queue,
};
use serde_json::{Value, json};

#[path = "support/recovery_readiness_fixture.rs"]
mod fixture;
#[path = "../../../tests/support/async_orphan_fixture.rs"]
mod orphan;
use fixture::{Fixture, ID, JOB, RUNTIME, SIBLING, TARGET, observation};

#[test]
fn exact_historical_evidence_is_read_only_and_never_releases_old_jobs() {
    let f = Fixture::new(a::Decision::AbandonOnly);
    let observed = f.settle("original");
    let before = f.state();
    let bytes = std::fs::read(&f.path).unwrap();
    let snapshot = r::capture(&f.path, ID, 1).unwrap();
    assert_eq!(snapshot.thread_id(), TARGET);
    assert_eq!(snapshot.turn_ids(), ["original"]);
    let report = serde_json::to_value(
        r::verify_observation(&f.path, &snapshot, &observed, RUNTIME, 1).unwrap(),
    )
    .unwrap();
    assert_eq!(report["release_authorized"], false);
    assert_eq!(report["execution_authorized"], false);
    assert_eq!(report["thread_id"], TARGET);
    assert_eq!(report["disposition_id"], ID);
    assert_eq!(report["evidence_sha256"].as_str().unwrap().len(), 64);
    for private in [
        "private preserved input",
        "original context",
        "only this answer",
    ] {
        assert!(!report.to_string().contains(private));
    }
    assert_eq!(std::fs::read(&f.path).unwrap(), bytes);
    assert_eq!(f.state(), before);
    f.held();
    assert_eq!(queue::list(&f.path).unwrap()[0].job_id, SIBLING);
    assert!(a::decision_status(&f.path, ID, 1).unwrap().is_some());
}

#[test]
fn missing_terminal_and_answer_receipt_alone_do_not_become_execution_proof() {
    let f = Fixture::new(a::Decision::AbandonOnly);
    aq::confirm_dispatch(&f.path, &f.question, "original").unwrap();
    queue::complete(&f.path, "origin").unwrap();
    let before = f.state();
    assert!(r::capture(&f.path, ID, 1).is_err());
    assert_eq!(f.state(), before);
    f.held();
}

#[test]
fn keep_held_or_wrong_disposition_revision_has_no_abandonment_evidence() {
    let f = Fixture::new(a::Decision::KeepHeld);
    f.settle("original");
    let before = f.state();
    assert!(r::capture(&f.path, ID, 1).is_err());
    assert!(r::capture(&f.path, ID, 2).is_err());
    assert_eq!(f.state(), before);
    assert!(
        queue::list(&f.path)
            .unwrap()
            .iter()
            .any(|job| job.job_id == JOB)
    );
}

#[test]
fn goal_handoff_requires_the_current_successor_not_the_original_terminal() {
    let f = Fixture::new(a::Decision::AbandonOnly);
    f.successor();
    let observed = f.settle("successor");
    let snapshot = r::capture(&f.path, ID, 1).unwrap();
    assert_eq!(snapshot.turn_ids(), ["successor"]);
    assert!(
        r::verify_observation(&f.path, &snapshot, &observation("original"), RUNTIME, 1).is_err()
    );
    r::verify_observation(&f.path, &snapshot, &observed, RUNTIME, 1).unwrap();
    f.held();
}

#[test]
fn unknown_goal_thread_or_turn_observation_is_never_a_positive_report() {
    let f = Fixture::new(a::Decision::AbandonOnly);
    let observed = f.settle("original");
    let snapshot = r::capture(&f.path, ID, 1).unwrap();
    let mut cases = Vec::new();
    for (pointer, value) in [
        ("/goal_observation", json!({})),
        (
            "/goal_observation/goal",
            json!({"threadId":TARGET,"status":"active"}),
        ),
        (
            "/goal_observation/goal",
            json!({"threadId":"other","status":"complete"}),
        ),
        ("/threadId", json!("other")),
        ("/thread_observation/thread/id", json!("other")),
        ("/thread_observation/thread/status", Value::Null),
        ("/thread_observation/thread/status/type", json!("active")),
        ("/turns/0/status", json!("inProgress")),
        ("/truncated", json!(true)),
    ] {
        let mut changed = observed.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        cases.push(changed);
    }
    let mut duplicate = observed.clone();
    duplicate["turns"]
        .as_array_mut()
        .unwrap()
        .push(observed["turns"][0].clone());
    cases.push(duplicate);
    for changed in cases {
        assert!(r::verify_observation(&f.path, &snapshot, &changed, RUNTIME, 1).is_err());
    }
    assert!(r::verify_observation(&f.path, &snapshot, &observed, "other-resident", 1).is_err());
    assert!(r::verify_observation(&f.path, &snapshot, &observed, RUNTIME, 0).is_err());
    f.held();
}

#[test]
fn changed_mapping_runtime_or_pending_inventory_rejects_a_stale_snapshot() {
    for sql in [
        "UPDATE mirror_threads SET discord_thread_id=21 WHERE codex_thread_id='thread-b'",
        "UPDATE codex_app_server_runtime SET runtime_id='new-app'",
        "UPDATE codex_mutation_runtime SET runtime_id='new-wire'",
        "UPDATE codex_turn_queue SET prompt='changed old input' WHERE job_id='bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'",
    ] {
        let f = Fixture::new(a::Decision::AbandonOnly);
        let observed = f.settle("original");
        let snapshot = r::capture(&f.path, ID, 1).unwrap();
        f.db.execute(sql, []).unwrap();
        let bytes = std::fs::read(&f.path).unwrap();
        assert!(
            r::verify_observation(&f.path, &snapshot, &observed, RUNTIME, 1).is_err(),
            "{sql}"
        );
        assert_eq!(std::fs::read(&f.path).unwrap(), bytes);
    }
}

#[test]
fn archive_and_prepared_wire_fences_are_independent_of_the_disposition() {
    for sql in [
        "INSERT INTO codex_archive_fences VALUES('thread-b','archive-a',NULL,'attempted')",
        "INSERT INTO codex_archive_fences VALUES('thread-b','archive-a',NULL,'verified')",
        "INSERT INTO codex_mutation_attempts
         (attempt_id,runtime_id,owner_id,generation,wire_id,method,target_thread_id,scoped,request_sha256,state,created_at,updated_at)
         VALUES('wire-a','wire-fixture','owner',1,'rpc-a','turn/start','thread-b',1,'digest','prepared',20,20)",
    ] {
        let f = Fixture::new(a::Decision::AbandonOnly);
        let observed = f.settle("original");
        let snapshot = r::capture(&f.path, ID, 1).unwrap();
        f.db.execute_batch(sql).unwrap();
        let before = f.state();
        assert!(r::capture(&f.path, ID, 1).is_err(), "{sql}");
        assert!(r::verify_observation(&f.path, &snapshot, &observed, RUNTIME, 1).is_err());
        assert_eq!(f.state(), before);
    }
}

#[test]
fn unfinished_stop_or_unclassifiable_ingress_stays_held() {
    for payload in [
        json!({"version":1,"plan":{"Execute":{"Stop":{"reference":null}}}}),
        json!({}),
    ] {
        let f = Fixture::new(a::Decision::AbandonOnly);
        f.settle("original");
        f.db.execute(
            "INSERT INTO discord_ingress_journal
            (ingress_id,kind,event_id,channel_id,owner_user_id,source_message_id,payload_json,
             state,phase,target_thread_id,created_at,updated_at)
            VALUES('message:200','message',200,20,30,200,?,'held','processing','thread-b',20,20)",
            [payload.to_string()],
        )
        .unwrap();
        let before = f.state();
        assert!(r::capture(&f.path, ID, 1).is_err());
        assert_eq!(f.state(), before);
    }
}

#[test]
fn future_capability_or_broken_schema_fails_read_only_without_migration() {
    for sql in [
        "UPDATE cdr_runtime_capability_requirements SET format_version=2 WHERE component='recovery_admission_order'",
        "DROP TABLE cdr_async_terminal_settlements",
    ] {
        let f = Fixture::new(a::Decision::AbandonOnly);
        f.settle("original");
        f.db.execute_batch(sql).unwrap();
        let bytes = std::fs::read(&f.path).unwrap();
        assert!(r::capture(&f.path, ID, 1).is_err());
        assert_eq!(std::fs::read(&f.path).unwrap(), bytes);
    }
}

#[test]
fn cold_new_resident_rechecks_current_facts_without_reusing_old_snapshot() {
    let f = Fixture::new(a::Decision::AbandonOnly);
    let observed = f.settle("original");
    let old = r::capture(&f.path, ID, 1).unwrap();
    f.db.execute(
        "UPDATE codex_app_server_runtime SET runtime_id='new-app'",
        [],
    )
    .unwrap();
    f.db.execute(
        "UPDATE codex_mutation_runtime SET runtime_id='new-wire'",
        [],
    )
    .unwrap();
    assert!(r::verify_observation(&f.path, &old, &observed, "new-app", 2).is_err());
    let current = r::capture(&f.path, ID, 1).unwrap();
    r::verify_observation(&f.path, &current, &observed, "new-app", 2).unwrap();
    f.held();
}

#[test]
fn unrelated_target_changes_do_not_rewrite_the_original_evidence() {
    let f = Fixture::new(a::Decision::AbandonOnly);
    let observed = f.settle("original");
    let snapshot = r::capture(&f.path, ID, 1).unwrap();
    f.db.execute(
        "INSERT INTO mirror_threads VALUES('other','project','other',10,21,0)",
        [],
    )
    .unwrap();
    queue::enqueue(
        &f.path,
        queue::NewQueueJob {
            job_id: "other-job",
            target_thread_id: "other",
            channel_id: 21,
            owner_user_id: Some(31),
            discord_message_id: Some(300),
            app_server_generation: 1,
            prompt: "unrelated new input",
            queued: true,
            ack_sent: true,
            created_at: 25.0,
        },
    )
    .unwrap();
    r::verify_observation(&f.path, &snapshot, &observed, RUNTIME, 1).unwrap();
    assert!(
        queue::try_begin_attempt(&f.path, "other-job", &[], 1)
            .unwrap()
            .is_some()
    );
    f.held();
}

#[test]
fn absent_database_and_copied_install_do_not_manufacture_local_identity() {
    let f = Fixture::new(a::Decision::AbandonOnly);
    f.settle("original");
    let missing = f.temp.path().join("missing.sqlite");
    assert!(r::capture(&missing, ID, 1).is_err());
    assert!(!missing.exists());
    let copied = f.temp.path().join("copied.sqlite");
    std::fs::copy(&f.path, &copied).unwrap();
    let bytes = std::fs::read(&copied).unwrap();
    assert!(r::capture(&copied, ID, 1).is_err());
    assert_eq!(std::fs::read(&copied).unwrap(), bytes);
}
