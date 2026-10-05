use cdr_store::async_resolution::abandonment as a;
use rusqlite::{Connection, params, types::ValueRef};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[path = "support/recovery_abandonment_fixture.rs"]
mod fixture;
use fixture::{Fixture, ID, JOB};

const NEXT_ID: &str = "dddddddddddddddddddddddddddddddd";

fn private_rows(db: &Connection, sql: &str) -> Vec<Vec<Value>> {
    let mut statement = db.prepare(sql).unwrap();
    let columns = statement.column_count();
    let mut cursor = statement.query([]).unwrap();
    let mut result = Vec::new();
    while let Some(row) = cursor.next().unwrap() {
        let mut cells = Vec::new();
        for index in 0..columns {
            cells.push(match row.get_ref(index).unwrap() {
                ValueRef::Null => json!(["null"]),
                ValueRef::Integer(v) => json!(["integer", v]),
                ValueRef::Real(v) => json!(["real_bits", v.to_bits().to_string()]),
                ValueRef::Text(v) => json!(["text", std::str::from_utf8(v).unwrap()]),
                ValueRef::Blob(v) => json!(["blob_hex", hex::encode(v)]),
            });
        }
        result.push(cells);
    }
    result
}

// Prepare a separately sealed row without altering any installed protection.
// The positive control below validates this row through the public binding API.
fn stage_next_revision(f: &Fixture) -> String {
    assert!(f.path.starts_with(f.temp.path()));
    let original: String =
        f.db.query_row(
            "SELECT seal_json FROM cdr_recovery_abandonment_proposals WHERE id=?",
            [ID],
            |row| row.get(0),
        )
        .unwrap();
    let mut next: Value = serde_json::from_str(&original).unwrap();
    let review = next["proposal"]["review_text"]
        .as_str()
        .unwrap()
        .replace("Proposal revision: 1", "Proposal revision: 2");
    assert!(review.contains("Proposal revision: 2"));
    let review_sha = hex::encode(Sha256::digest(review.as_bytes()));
    next["proposal"]["id"] = json!(NEXT_ID);
    next["proposal"]["revision"] = json!(2);
    next["proposal"]["review_text"] = json!(review);
    next["proposal"]["review_sha256"] = json!(review_sha);
    let encoded = serde_json::to_string(&next).unwrap();
    let seal_sha = hex::encode(Sha256::digest(encoded.as_bytes()));
    f.db.execute_batch(
        "CREATE TABLE fixture_next_abandonment AS
         SELECT * FROM cdr_recovery_abandonment_proposals WHERE 0",
    )
    .unwrap();
    assert_eq!(
        f.db.execute(
            "INSERT INTO fixture_next_abandonment
         SELECT ?1,format_version,2,job_id,target_thread_id,owner_user_id,
                channel_id,application_id,?2,?3
         FROM cdr_recovery_abandonment_proposals WHERE id=?4",
            params![NEXT_ID, encoded, seal_sha, ID],
        )
        .unwrap(),
        1
    );
    review_sha
}

fn assert_postwrite_revision_rolls_back(event: &str, predicate: &str, decision: a::Decision) {
    let f = Fixture::new();
    let original = f.delivered();
    f.click(decision);
    stage_next_revision(&f);
    let queued = private_rows(&f.db, "SELECT * FROM codex_turn_queue ORDER BY job_id");
    let proposals = private_rows(
        &f.db,
        "SELECT * FROM cdr_recovery_abandonment_proposals ORDER BY id",
    );
    let deliveries = private_rows(
        &f.db,
        "SELECT * FROM cdr_recovery_abandonment_deliveries ORDER BY proposal_id",
    );
    let ingress = private_rows(
        &f.db,
        "SELECT * FROM discord_ingress_journal ORDER BY ingress_id",
    );
    f.db.execute_batch(&format!(
        "CREATE TRIGGER fixture_postwrite_revision AFTER {event}
         WHEN {predicate}
         BEGIN
             INSERT INTO cdr_recovery_abandonment_proposals
             SELECT * FROM fixture_next_abandonment;
         END"
    ))
    .unwrap();

    let result = f.apply(decision);
    assert!(
        result.is_err(),
        "stale revision committed after {event}: {result:?}"
    );
    f.unchanged();
    assert_eq!(
        private_rows(&f.db, "SELECT * FROM codex_turn_queue ORDER BY job_id"),
        queued
    );
    assert_eq!(
        private_rows(
            &f.db,
            "SELECT * FROM cdr_recovery_abandonment_proposals ORDER BY id"
        ),
        proposals
    );
    assert_eq!(
        private_rows(
            &f.db,
            "SELECT * FROM cdr_recovery_abandonment_deliveries ORDER BY proposal_id"
        ),
        deliveries
    );
    assert_eq!(
        private_rows(
            &f.db,
            "SELECT * FROM discord_ingress_journal ORDER BY ingress_id"
        ),
        ingress
    );
    assert_eq!(f.count("cdr_recovery_abandonment_proposals"), 1);
    assert_eq!(f.count("fixture_next_abandonment"), 1);
    assert_eq!(
        a::delivered_proposal(&f.path, ID, 1).unwrap().proposal,
        original
    );
}

#[test]
fn revision_added_after_decision_insert_rolls_back_abandonment() {
    assert_postwrite_revision_rolls_back(
        "INSERT ON cdr_recovery_abandonment_decisions",
        &format!("NEW.proposal_id='{ID}'"),
        a::Decision::AbandonOnly,
    );
}

#[test]
fn revision_added_after_cancellation_insert_rolls_back_abandonment() {
    assert_postwrite_revision_rolls_back(
        "INSERT ON codex_request_cancellations",
        &format!("NEW.job_id='{JOB}'"),
        a::Decision::AbandonOnly,
    );
}

#[test]
fn revision_added_after_original_delete_rolls_back_abandonment() {
    assert_postwrite_revision_rolls_back(
        "DELETE ON codex_turn_queue",
        &format!("OLD.job_id='{JOB}'"),
        a::Decision::AbandonOnly,
    );
}

#[test]
fn keep_held_still_checks_the_postwrite_revision() {
    assert_postwrite_revision_rolls_back(
        "INSERT ON cdr_recovery_abandonment_decisions",
        &format!("NEW.proposal_id='{ID}'"),
        a::Decision::KeepHeld,
    );
}

#[test]
fn superseding_fixture_is_a_valid_seal_and_preexisting_supersede_is_rejected() {
    let f = Fixture::new();
    f.delivered();
    f.click(a::Decision::AbandonOnly);
    let review_sha = stage_next_revision(&f);
    f.db.execute(
        "INSERT INTO cdr_recovery_abandonment_proposals SELECT * FROM fixture_next_abandonment",
        [],
    )
    .unwrap();
    a::bind_delivery(&f.path, NEXT_ID, 61, &review_sha, 12.0).unwrap();
    let current = a::delivered_proposal(&f.path, NEXT_ID, 2).unwrap();
    assert_eq!(current.proposal.revision, 2);
    assert_eq!(current.proposal.job_id, JOB);
    assert_eq!(current.proposal.review_sha256, review_sha);
    assert!(f.apply(a::Decision::AbandonOnly).is_err());
    f.unchanged();
    assert_eq!(f.count("cdr_recovery_abandonment_proposals"), 2);
}

#[test]
fn committed_original_click_does_not_require_the_latest_revision() {
    let f = Fixture::new();
    f.delivered();
    f.click(a::Decision::AbandonOnly);
    stage_next_revision(&f);
    let committed = f.apply(a::Decision::AbandonOnly).unwrap();
    f.db.execute(
        "INSERT INTO cdr_recovery_abandonment_proposals SELECT * FROM fixture_next_abandonment",
        [],
    )
    .unwrap();
    assert_eq!(f.apply(a::Decision::AbandonOnly).unwrap(), committed);
    assert_eq!(a::decision_status(&f.path, ID, 1).unwrap(), Some(committed));
    assert_eq!(f.count("cdr_recovery_abandonment_decisions"), 1);
    assert_eq!(f.count("codex_request_cancellations"), 1);
    assert_eq!(f.count("codex_turn_queue"), 1);
}
