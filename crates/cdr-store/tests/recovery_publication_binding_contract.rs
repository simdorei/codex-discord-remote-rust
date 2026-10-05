use std::path::PathBuf;

use cdr_store::{
    async_resolution::{self, publication as p},
    queue,
    schema::open_initialized,
};
use rusqlite::Connection;
use serde_json::json;

const ID: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const TARGET: &str = async_resolution::REVIEWED_INCIDENT_THREAD;

struct Fixture {
    _temp: tempfile::TempDir,
    path: PathBuf,
    proposal: p::Proposal,
}

impl Fixture {
    fn new(deliver: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("store.sqlite");
        cdr_store::mapping::upsert_thread(&path, TARGET, "project", "title", 10, 20, 1.0).unwrap();
        queue::enqueue(
            &path,
            queue::NewQueueJob {
                job_id: "pending",
                target_thread_id: TARGET,
                channel_id: 20,
                owner_user_id: Some(30),
                discord_message_id: None,
                app_server_generation: 1,
                prompt: "exact fixture input",
                queued: true,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        let proposal = p::propose(
            &path,
            &p::ProposalInput {
                proposal_id: ID,
                job_id: "pending",
                application_id: 50,
                review_text: "Intent only; nothing is started.",
                review_context: &json!({"fixture":true}),
                now: 10.0,
                expires_at: 100.0,
            },
        )
        .unwrap();
        if deliver {
            p::bind_delivery(&path, ID, 60, &proposal.review_sha256, 11.0).unwrap();
        }
        Self {
            _temp: temp,
            path,
            proposal,
        }
    }
}

#[test]
fn exact_delivery_binding_is_readonly_and_rejects_other_authenticated_headers() {
    let f = Fixture::new(true);
    let before = std::fs::read(&f.path).unwrap();
    let binding = p::delivered_proposal(&f.path, ID, 1).unwrap();
    assert_eq!(binding.proposal, f.proposal);
    assert_eq!(binding.message_id, 60);
    binding.require_actor(50, 20, 30, 60).unwrap();
    for (app, channel, actor, message) in [
        (51, 20, 30, 60),
        (50, 21, 30, 60),
        (50, 20, 31, 60),
        (50, 20, 30, 61),
        (50, 20, 30, 0),
    ] {
        assert!(binding.require_actor(app, channel, actor, message).is_err());
    }
    assert_eq!(before, std::fs::read(&f.path).unwrap());
}

#[test]
fn missing_unbound_stale_or_malformed_delivery_identity_cannot_route() {
    let f = Fixture::new(false);
    assert!(p::delivered_proposal(&f.path, ID, 1).is_err());
    p::bind_delivery(&f.path, ID, 60, &f.proposal.review_sha256, 11.0).unwrap();
    for (id, revision) in [
        (ID, 0),
        (ID, 2),
        ("abc", 1),
        ("CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC", 1),
    ] {
        assert!(p::delivered_proposal(&f.path, id, revision).is_err());
    }
    let missing = f.path.with_file_name("absent.sqlite");
    assert!(p::delivered_proposal(&missing, ID, 1).is_err());
    assert!(!missing.exists());
}

#[test]
fn unsupported_or_incomplete_readonly_database_is_never_migrated() {
    for change in [
        "UPDATE cdr_runtime_capability_requirements SET format_version=2 WHERE component='recovery_publication_consent'",
        "ALTER TABLE cdr_recovery_publication_proposals RENAME COLUMN seal_json TO legacy_seal_json",
        "DROP TABLE cdr_recovery_publication_deliveries",
    ] {
        let f = Fixture::new(true);
        open_initialized(&f.path)
            .unwrap()
            .execute_batch(change)
            .unwrap();
        let before = std::fs::read(&f.path).unwrap();
        assert!(p::delivered_proposal(&f.path, ID, 1).is_err());
        assert_eq!(before, std::fs::read(&f.path).unwrap());
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("legacy.sqlite");
    Connection::open(&path)
        .unwrap()
        .execute_batch("PRAGMA user_version=2")
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(p::delivered_proposal(&path, ID, 1).is_err());
    assert_eq!(before, std::fs::read(&path).unwrap());
}

#[test]
fn historical_delivery_binding_does_not_refresh_snapshot_or_release_pending() {
    let f = Fixture::new(true);
    open_initialized(&f.path)
        .unwrap()
        .execute(
            "UPDATE codex_turn_queue SET prompt='changed evidence' WHERE job_id='pending'",
            [],
        )
        .unwrap();
    let before = std::fs::read(&f.path).unwrap();
    assert_eq!(
        p::delivered_proposal(&f.path, ID, 1).unwrap().proposal,
        f.proposal
    );
    assert_eq!(before, std::fs::read(&f.path).unwrap());
    assert!(async_resolution::admission_held(&f.path, TARGET).unwrap());
    assert!(queue::try_begin_attempt(&f.path, "pending", &[], 1).is_err());
    let count: i64 = open_initialized(&f.path)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM cdr_recovery_publication_decisions",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}
