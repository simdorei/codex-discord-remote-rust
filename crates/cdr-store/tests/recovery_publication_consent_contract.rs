use std::path::PathBuf;

use cdr_store::{
    async_resolution::{self, publication as p},
    queue,
    schema::open_initialized,
};
use rusqlite::{Connection, params};
use serde_json::json;

const TARGET: &str = async_resolution::REVIEWED_INCIDENT_THREAD;
const PROPOSAL: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[path = "recovery_publication_consent_contract/failures.rs"]
mod failures;

struct Fixture {
    _directory: tempfile::TempDir,
    path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.sqlite");
        let db = open_initialized(&path).unwrap();
        db.execute(
            "INSERT INTO mirror_threads VALUES (?,'project','title',10,20,0)",
            [TARGET],
        )
        .unwrap();
        queue::enqueue(
            &path,
            queue::NewQueueJob {
                job_id: "pending",
                target_thread_id: TARGET,
                channel_id: 20,
                owner_user_id: Some(30),
                discord_message_id: None,
                app_server_generation: 1,
                prompt: "fixture-only exact pending",
                queued: true,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        Self {
            _directory: directory,
            path,
        }
    }

    fn db(&self) -> Connection {
        open_initialized(&self.path).unwrap()
    }

    fn propose(&self, id: &str) -> cdr_store::Result<p::Proposal> {
        p::propose(
            &self.path,
            &p::ProposalInput {
                proposal_id: id,
                job_id: "pending",
                application_id: 50,
                review_text: "Fixture review only. No execution or publication grant.",
                review_context: &json!({"fixture":true,"publisher_exclusion":"unverified"}),
                now: 10.0,
                expires_at: 100.0,
            },
        )
    }

    fn delivered(&self, id: &str, message: i64) -> p::Proposal {
        let proposal = self.propose(id).unwrap();
        p::bind_delivery(&self.path, id, message, &proposal.review_sha256, 11.0).unwrap();
        proposal
    }

    fn click(&self, proposal: &p::Proposal, event: i64, message: i64, choice: &str) -> String {
        let ingress = format!("fixture-interaction:{event}");
        let payload = json!({"version":1,"work":{"Component":{"RecoveryPublicationDecision":{
            "proposal_id":proposal.id,"revision":proposal.revision,"decision":choice,
        }}}});
        self.db().execute(
            "INSERT INTO discord_ingress_journal
             (ingress_id,kind,event_id,application_id,channel_id,owner_user_id,source_message_id,
              payload_json,runtime_id,state,phase,target_thread_id,created_at,updated_at)
             VALUES(?,'interaction',?,50,20,30,?,?,'fixture-runtime','executing','processing',?,12,12)",
            params![ingress,event,message,payload.to_string(),TARGET],
        ).unwrap();
        ingress
    }

    fn record(
        &self,
        proposal: &p::Proposal,
        ingress: &str,
        now: f64,
    ) -> cdr_store::Result<p::DecisionReceipt> {
        p::record_consent(
            &self.path,
            &p::ConsentInput {
                proposal_id: &proposal.id,
                revision: proposal.revision,
                ingress_id: ingress,
                now,
            },
        )
    }

    fn count(&self, table: &str) -> i64 {
        self.db()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    fn assert_no_execution(&self) {
        let state: (String,i64,Option<String>,String) = self.db().query_row(
            "SELECT state,attempt_count,turn_id,prompt FROM codex_turn_queue WHERE job_id='pending'",
            [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).unwrap();
        assert_eq!(
            state,
            (
                "pending".into(),
                0,
                None,
                "fixture-only exact pending".into()
            )
        );
        assert!(async_resolution::admission_held(&self.path, TARGET).unwrap());
        assert!(queue::try_begin_attempt(&self.path, "pending", &[], 1).is_err());
    }
}

#[test]
fn exact_consent_survives_cold_reads_without_authorizing_execution() {
    let f = Fixture::new();
    let proposal = f.delivered(PROPOSAL, 60);
    let click = f.click(&proposal, 70, 60, "ApproveExact");
    let first = f.record(&proposal, &click, 12.0).unwrap();
    assert_eq!(first.decision, p::Decision::ApproveExact);
    assert!(!first.already_recorded);
    f.db()
        .execute(
            "UPDATE discord_ingress_journal SET state='completed' WHERE ingress_id=?",
            [&click],
        )
        .unwrap();
    let again = f.record(&proposal, &click, 101.0).unwrap();
    assert!(again.already_recorded);
    assert_eq!(again.original_interaction_id, 70);
    assert_eq!(f.count("cdr_recovery_publication_decisions"), 1);
    f.assert_no_execution();
}

#[test]
fn second_click_keeps_original_receipt_and_conflicting_decision_is_rejected() {
    let f = Fixture::new();
    let proposal = f.delivered(PROPOSAL, 60);
    let first = f.click(&proposal, 70, 60, "ApproveExact");
    f.record(&proposal, &first, 12.0).unwrap();
    let again = f.click(&proposal, 71, 60, "ApproveExact");
    assert_eq!(
        f.record(&proposal, &again, 13.0)
            .unwrap()
            .original_ingress_id,
        first
    );
    let conflict = f.click(&proposal, 72, 60, "KeepHeld");
    assert!(f.record(&proposal, &conflict, 14.0).is_err());
    assert_eq!(f.count("cdr_recovery_publication_decisions"), 1);
    f.assert_no_execution();
}

#[test]
fn keep_held_is_an_immutable_decision_not_a_replay_or_cancellation() {
    let f = Fixture::new();
    let proposal = f.delivered(PROPOSAL, 60);
    let click = f.click(&proposal, 70, 60, "KeepHeld");
    assert_eq!(
        f.record(&proposal, &click, 12.0).unwrap().decision,
        p::Decision::KeepHeld
    );
    f.assert_no_execution();
}

#[test]
fn exact_actor_application_message_and_live_custody_are_required() {
    for assignment in [
        "owner_user_id=31",
        "channel_id=21",
        "application_id=51",
        "application_id=NULL",
        "source_message_id=61",
        "source_message_id=NULL",
        "target_thread_id='other'",
        "kind='message'",
        "event_id=NULL",
        "state='held'",
        "state='completed'",
        "phase='unknown'",
        "runtime_id=NULL",
        "runtime_id=' '",
        "owner_kind='queue'",
        "owner_id='other'",
    ] {
        let f = Fixture::new();
        let proposal = f.delivered(PROPOSAL, 60);
        let click = f.click(&proposal, 70, 60, "ApproveExact");
        f.db()
            .execute(
                &format!("UPDATE discord_ingress_journal SET {assignment} WHERE ingress_id=?"),
                [&click],
            )
            .unwrap();
        assert!(f.record(&proposal, &click, 12.0).is_err(), "{assignment}");
        assert_eq!(f.count("cdr_recovery_publication_decisions"), 0);
        f.assert_no_execution();
    }
}

#[test]
fn malformed_or_different_component_cannot_borrow_durable_custody() {
    for payload in [
        json!({}),
        json!({"version":2,"work":{"Component":{"RecoveryPublicationDecision":{
            "proposal_id":PROPOSAL,"revision":1,"decision":"ApproveExact"}}}}),
        json!({"version":1,"work":{"Component":{"RecoveryPublicationDecision":{
            "proposal_id":PROPOSAL,"revision":2,"decision":"ApproveExact"}}}}),
        json!({"version":1,"work":{"Component":{"RecoveryPublicationDecision":{
            "proposal_id":PROPOSAL,"revision":1,"decision":"ApproveExact","extra":true}}}}),
        json!({"version":1,"work":{"Component":{"BoundApproval":{"answer":"ApproveSession"}}}}),
    ] {
        let f = Fixture::new();
        let proposal = f.delivered(PROPOSAL, 60);
        let click = f.click(&proposal, 70, 60, "ApproveExact");
        f.db()
            .execute(
                "UPDATE discord_ingress_journal SET payload_json=? WHERE ingress_id=?",
                params![payload.to_string(), click],
            )
            .unwrap();
        assert!(f.record(&proposal, &click, 12.0).is_err());
        assert_eq!(f.count("cdr_recovery_publication_decisions"), 0);
    }
}

#[test]
fn changed_pending_bytes_nulls_and_timestamp_bits_invalidate_consent() {
    for assignment in [
        "prompt='different input'",
        "discord_message_id=99",
        "baseline_turn_ids='[ ]'",
        "last_error='new evidence'",
        "app_server_generation=2",
        "owner_user_id=31",
        "updated_at=1.0000000000000002",
        "created_at=1.0000000000000002",
        "queued=0",
        "ack_sent=0",
    ] {
        let f = Fixture::new();
        let proposal = f.delivered(PROPOSAL, 60);
        let click = f.click(&proposal, 70, 60, "ApproveExact");
        f.db()
            .execute(
                &format!("UPDATE codex_turn_queue SET {assignment} WHERE job_id='pending'"),
                [],
            )
            .unwrap();
        assert!(f.record(&proposal, &click, 12.0).is_err(), "{assignment}");
        assert_eq!(f.count("cdr_recovery_publication_decisions"), 0);
    }
}

#[test]
fn remap_or_new_stop_revision_invalidates_the_original_proposal() {
    for stop in [false, true] {
        let f = Fixture::new();
        let proposal = f.delivered(PROPOSAL, 60);
        let click = f.click(&proposal, 70, 60, "ApproveExact");
        if stop {
            f.db()
                .execute_batch(&format!(
                "INSERT INTO cdr_stop_revision_receipts VALUES('fixture-stop','{TARGET}',1,'{{}}');
                 INSERT INTO cdr_stop_revisions VALUES('{TARGET}',1,'fixture-stop');
                 UPDATE cdr_stop_clock SET revision=1 WHERE singleton=1;"
            ))
                .unwrap();
        } else {
            f.db()
                .execute("UPDATE mirror_threads SET discord_thread_id=21", [])
                .unwrap();
        }
        assert!(f.record(&proposal, &click, 12.0).is_err());
        assert_eq!(f.count("cdr_recovery_publication_decisions"), 0);
    }
}

#[test]
fn a_new_revision_supersedes_old_ui_but_preserves_all_old_evidence() {
    let f = Fixture::new();
    let old = f.delivered(PROPOSAL, 60);
    let old_click = f.click(&old, 70, 60, "ApproveExact");
    let new = f.delivered("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", 61);
    assert_eq!(new.revision, 2);
    assert!(f.record(&old, &old_click, 12.0).is_err());
    let click = f.click(&new, 71, 61, "ApproveExact");
    f.record(&new, &click, 12.0).unwrap();
    assert_eq!(f.count("cdr_recovery_publication_proposals"), 2);
    assert_eq!(f.count("cdr_recovery_publication_deliveries"), 2);
    f.assert_no_execution();
}

#[test]
fn expiry_and_nonfinite_clock_never_create_a_new_decision() {
    for now in [9.0, 100.0, 101.0, f64::NAN, f64::INFINITY] {
        let f = Fixture::new();
        let proposal = f.delivered(PROPOSAL, 60);
        let click = f.click(&proposal, 70, 60, "ApproveExact");
        assert!(f.record(&proposal, &click, now).is_err());
        assert_eq!(f.count("cdr_recovery_publication_decisions"), 0);
    }
}

#[test]
fn unconfirmed_or_conflicting_delivery_does_not_enable_consent() {
    let f = Fixture::new();
    let proposal = f.propose(PROPOSAL).unwrap();
    let click = f.click(&proposal, 70, 60, "ApproveExact");
    assert!(f.record(&proposal, &click, 12.0).is_err());
    assert!(p::bind_delivery(&f.path, PROPOSAL, 60, &"0".repeat(64), 11.0).is_err());
    p::bind_delivery(&f.path, PROPOSAL, 60, &proposal.review_sha256, 11.0).unwrap();
    assert!(p::bind_delivery(&f.path, PROPOSAL, 61, &proposal.review_sha256, 11.0).is_err());
    f.record(&proposal, &click, 12.0).unwrap();
    f.assert_no_execution();
}

#[test]
fn lost_decision_insert_rolls_back_and_does_not_authorize_a_start() {
    for action in ["IGNORE", "ABORT,'injected failure'"] {
        let f = Fixture::new();
        let proposal = f.delivered(PROPOSAL, 60);
        let click = f.click(&proposal, 70, 60, "ApproveExact");
        f.db()
            .execute_batch(&format!(
            "CREATE TRIGGER injected_decision BEFORE INSERT ON cdr_recovery_publication_decisions
             BEGIN SELECT RAISE({action}); END;"
        ))
            .unwrap();
        assert!(f.record(&proposal, &click, 12.0).is_err());
        assert_eq!(f.count("cdr_recovery_publication_decisions"), 0);
        f.assert_no_execution();
    }
}

#[test]
fn post_insert_pending_change_rolls_back_both_decision_and_change() {
    let f = Fixture::new();
    let proposal = f.delivered(PROPOSAL, 60);
    let click = f.click(&proposal, 70, 60, "ApproveExact");
    f.db().execute_batch(
        "CREATE TRIGGER injected_change AFTER INSERT ON cdr_recovery_publication_decisions
         BEGIN UPDATE codex_turn_queue SET prompt='changed by trigger' WHERE job_id='pending'; END;"
    ).unwrap();
    assert!(f.record(&proposal, &click, 12.0).is_err());
    assert_eq!(f.count("cdr_recovery_publication_decisions"), 0);
    f.assert_no_execution();
}

#[test]
fn proposal_and_delivery_ignore_failures_cannot_report_success() {
    for table in [
        "cdr_recovery_publication_proposals",
        "cdr_recovery_publication_deliveries",
    ] {
        let f = Fixture::new();
        f.db()
            .execute_batch(&format!(
                "CREATE TRIGGER injected_ignore BEFORE INSERT ON {table}
             BEGIN SELECT RAISE(IGNORE); END;"
            ))
            .unwrap();
        if table.ends_with("proposals") {
            assert!(f.propose(PROPOSAL).is_err());
        } else {
            let proposal = f.propose(PROPOSAL).unwrap();
            assert!(
                p::bind_delivery(&f.path, PROPOSAL, 60, &proposal.review_sha256, 11.0).is_err()
            );
        }
        assert_eq!(f.count(table), 0);
        f.assert_no_execution();
    }
}

#[test]
fn identical_producer_is_idempotent_but_cannot_rebind_or_forget_evidence() {
    let f = Fixture::new();
    let proposal = f.delivered(PROPOSAL, 60);
    assert_eq!(proposal, f.propose(PROPOSAL).unwrap());
    let click = f.click(&proposal, 70, 60, "ApproveExact");
    f.record(&proposal, &click, 12.0).unwrap();
    for table in [
        "cdr_recovery_publication_proposals",
        "cdr_recovery_publication_deliveries",
        "cdr_recovery_publication_decisions",
    ] {
        assert!(f.db().execute(&format!("DELETE FROM {table}"), []).is_err());
        assert!(
            f.db()
                .execute(&format!("UPDATE {table} SET revision=revision+1"), [])
                .is_err()
        );
        assert_eq!(f.count(table), 1);
    }
    f.assert_no_execution();
}

#[test]
fn unsupported_capability_blocks_new_proposals_delivery_and_consent() {
    let f = Fixture::new();
    let proposal = f.propose(PROPOSAL).unwrap();
    let click = f.click(&proposal, 70, 60, "ApproveExact");
    f.db()
        .execute(
            "UPDATE cdr_runtime_capability_requirements SET format_version=2
         WHERE component='recovery_publication_consent'",
            [],
        )
        .unwrap();
    assert!(
        f.propose("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").is_err(),
        "unsupported proposal format"
    );
    assert!(p::bind_delivery(&f.path, PROPOSAL, 60, &proposal.review_sha256, 11.0).is_err());
    assert!(f.record(&proposal, &click, 12.0).is_err());
    assert_eq!(f.count("cdr_recovery_publication_decisions"), 0);
}
