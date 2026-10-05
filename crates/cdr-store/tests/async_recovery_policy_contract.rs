use cdr_store::{async_resolution, queue, schema::open_initialized};
use std::path::Path;

const INCIDENT: &str = "01a06156-56cd-70b0-af02-2de7445ba4c7";

fn pending(path: &Path, target: &str) {
    queue::enqueue(
        path,
        queue::NewQueueJob {
            job_id: "pending",
            target_thread_id: target,
            channel_id: 20,
            owner_user_id: Some(30),
            discord_message_id: None,
            app_server_generation: 1,
            prompt: "isolated recovery-policy fixture",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
}

#[test]
fn reviewed_incident_never_defaults_to_clean_ordinary_admission() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    open_initialized(&path).unwrap();
    assert!(!async_resolution::admission_held(&path, "unrelated").unwrap());
    assert!(
        async_resolution::admission_held(&path, INCIDENT).unwrap(),
        "incident admission escaped before its reviewed publishing policy was armed"
    );
}

#[test]
fn incident_cannot_claim_a_pending_attempt_before_bootstrap_policy() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    pending(&path, INCIDENT);
    let before = queue::list(&path).unwrap();
    assert!(
        queue::begin_attempt(&path, "pending", &[], 1).is_err(),
        "incident Pending became Starting without bootstrap policy"
    );
    assert_eq!(queue::list(&path).unwrap(), before);
}

#[test]
fn raw_starting_cas_cannot_bypass_unarmed_incident_protection() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    pending(&path, INCIDENT);
    let before = queue::list(&path).unwrap();
    assert!(
        open_initialized(&path)
            .unwrap()
            .execute(
                "UPDATE codex_turn_queue SET state='starting' WHERE job_id='pending'",
                [],
            )
            .is_err(),
        "the final SQL transition admitted an unarmed incident"
    );
    assert_eq!(queue::list(&path).unwrap(), before);
}

#[test]
fn unscoped_mutation_cannot_borrow_missing_policy_as_permission() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    open_initialized(&path).unwrap();
    assert!(async_resolution::guard_mutation(&path, "unrelated").is_ok());
    assert!(
        async_resolution::guard_mutation(&path, INCIDENT).is_err(),
        "absence of an obligation/policy row must not authorize incident mutation"
    );
}

fn legacy_obligation(path: &Path) {
    open_initialized(path)
        .unwrap()
        .execute(
            "INSERT INTO cdr_async_execution_obligations
         (question_id,thread_id,origin_job_id,turn_id,channel_id,format_version,revision,
          answer_state,execution_state,admission_state,policy,original_seal,claim_json,
          original_error,created_at,updated_at)
         VALUES('legacy',?,'old-job','old-turn',20,1,7,'unresolved','unresolved','held',
                'ordinary',NULL,'{}','original failure',1,1)",
            [INCIDENT],
        )
        .unwrap();
}

fn registrations(path: &Path) -> i64 {
    open_initialized(path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM cdr_async_recovery_policies",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn bootstrap_registration_is_durable_idempotent_and_never_execution_permission() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    for _ in 0..2 {
        async_resolution::install_reviewed_policy(&path).unwrap();
    }
    let db = open_initialized(&path).unwrap();
    assert!(async_resolution::reviewed_policy_installed_in(&db).unwrap());
    assert_eq!(registrations(&path), 1);
    assert!(async_resolution::admission_held(&path, INCIDENT).unwrap());
    assert!(async_resolution::guard_mutation(&path, INCIDENT).is_err());
    assert!(!async_resolution::admission_held(&path, "unrelated").unwrap());
    assert!(
        db.execute("DELETE FROM cdr_async_recovery_policies", [])
            .is_err()
    );
    assert!(
        db.execute(
            "UPDATE cdr_async_recovery_policies SET proposal_sha256=?",
            ["a".repeat(64)]
        )
        .is_err()
    );
}

#[test]
fn explicit_hold_promotion_preserves_unproven_legacy_execution_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    legacy_obligation(&path);
    let before = async_resolution::inspect(&path, INCIDENT)
        .unwrap()
        .remove(0);
    async_resolution::install_reviewed_policy(&path).unwrap();
    let after = async_resolution::inspect(&path, INCIDENT)
        .unwrap()
        .remove(0);
    assert_eq!(after.policy, "publishing_recovery");
    assert_eq!(after.admission_state, "held");
    assert_eq!(after.claim_sha256, before.claim_sha256);
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.answer_state, before.answer_state);
    assert_eq!(after.execution_state, before.execution_state);
    assert_eq!(after.original_error, "original failure");
    assert!(
        async_resolution::capture_history_snapshot(&path, INCIDENT).is_err(),
        "arming a safety policy must not turn a missing original seal into proof"
    );
}

#[test]
fn policy_promotion_failure_rolls_back_registration_and_only_holds_incident() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    legacy_obligation(&path);
    open_initialized(&path).unwrap().execute_batch(
        "CREATE TRIGGER reject_policy_promotion BEFORE UPDATE OF policy ON cdr_async_execution_obligations
         BEGIN SELECT RAISE(ABORT,'injected policy promotion failure'); END;",
    ).unwrap();
    assert!(async_resolution::install_reviewed_policy(&path).is_err());
    assert_eq!(registrations(&path), 0);
    let row = async_resolution::inspect(&path, INCIDENT)
        .unwrap()
        .remove(0);
    assert_eq!(row.policy, "ordinary");
    assert_eq!(row.revision, 7);
    assert_eq!(row.original_error, "original failure");
    assert!(async_resolution::admission_held(&path, INCIDENT).unwrap());
    pending(&path, "unrelated");
    assert!(queue::begin_attempt(&path, "pending", &[], 1).is_ok());
}

#[test]
fn changed_or_ignored_registration_is_not_reported_as_armed() {
    for mode in ["ignore", "changed"] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.sqlite");
        let db = open_initialized(&path).unwrap();
        if mode == "ignore" {
            db.execute_batch(
                "CREATE TRIGGER ignore_policy BEFORE INSERT ON cdr_async_recovery_policies
                BEGIN SELECT RAISE(IGNORE); END;",
            )
            .unwrap();
        } else {
            db.execute("INSERT INTO cdr_async_recovery_policies VALUES(?,1,'publishing_recovery',?,'turn','job','pending')",
                rusqlite::params![INCIDENT,"a".repeat(64)]).unwrap();
        }
        assert!(
            async_resolution::install_reviewed_policy(&path).is_err(),
            "{mode}"
        );
        assert!(!async_resolution::reviewed_policy_installed_in(&db).unwrap());
        assert_eq!(registrations(&path), i64::from(mode == "changed"));
        assert!(async_resolution::admission_held(&path, INCIDENT).unwrap());
        assert!(async_resolution::guard_mutation(&path, "unrelated").is_ok());
    }
}

#[test]
fn unsupported_legacy_policy_is_preserved_without_partial_registration() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    legacy_obligation(&path);
    open_initialized(&path)
        .unwrap()
        .execute(
            "UPDATE cdr_async_execution_obligations SET policy='future_policy'",
            [],
        )
        .unwrap();
    assert!(async_resolution::install_reviewed_policy(&path).is_err());
    assert_eq!(registrations(&path), 0);
    assert_eq!(
        async_resolution::inspect(&path, INCIDENT).unwrap()[0].policy,
        "future_policy"
    );
    assert!(async_resolution::admission_held(&path, INCIDENT).unwrap());
}

#[path = "../../../tests/support/async_orphan_fixture.rs"]
mod legacy_fixture;

#[test]
fn extension_upgrade_refreshes_legacy_queue_delete_policy_capture() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("legacy-source.sqlite");
    let id = legacy_fixture::dispatching(&source, "old-resident");
    cdr_store::async_question::confirm_dispatch(&source, &id, "original").unwrap();
    legacy_fixture::pending(&source, "next", "thread-b", 1);
    let path = temp.path().join("upgraded.sqlite");
    let db = open_initialized(&path).unwrap();
    // Copy real public-API source rows, but not the later obligation ledger.
    // Historical submitted questions are deliberately not migration-backfilled.
    db.execute(
        "ATTACH DATABASE ? AS legacy_source",
        [source.to_string_lossy().as_ref()],
    )
    .unwrap();
    db.execute_batch(
        "INSERT INTO mirror_threads SELECT * FROM legacy_source.mirror_threads;
         INSERT INTO codex_turn_queue SELECT * FROM legacy_source.codex_turn_queue;
         INSERT INTO cdr_async_questions SELECT * FROM legacy_source.cdr_async_questions;
         DETACH DATABASE legacy_source;",
    )
    .unwrap();
    db.execute(
        "INSERT INTO cdr_async_recovery_policies VALUES('thread-b',1,'publishing_recovery',?,'original','origin','next')",
        [async_resolution::REVIEWED_PROPOSAL_SHA256],
    ).unwrap();
    let current: String = db
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='cdr_async_obligation_queue_delete'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let case = format!(
        "CASE WHEN q.thread_id='{}' OR EXISTS(\n        SELECT 1 FROM cdr_async_recovery_policies p WHERE p.thread_id=q.thread_id)\n        THEN 'publishing_recovery' ELSE 'ordinary' END",
        async_resolution::REVIEWED_INCIDENT_THREAD
    );
    let legacy = current.replace(&case, "'ordinary'");
    assert_ne!(
        legacy, current,
        "legacy-trigger fixture replacement did not match"
    );
    db.execute_batch(&format!(
        "DROP TRIGGER cdr_async_obligation_queue_delete; {legacy};
         DROP TRIGGER cdr_async_recovery_policy_capability;"
    ))
    .unwrap();
    drop(db);
    let db = open_initialized(&path).unwrap();
    let before: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM cdr_async_execution_obligations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        before, 0,
        "migration must not invent a historical submitted obligation"
    );
    db.execute("DELETE FROM codex_turn_queue WHERE job_id='origin'", [])
        .unwrap();
    let policy: String = db
        .query_row(
            "SELECT policy FROM cdr_async_execution_obligations WHERE question_id=?",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        policy, "publishing_recovery",
        "upgraded deletion trigger retained the old ordinary literal"
    );
    assert!(async_resolution::admission_held(&path, "thread-b").unwrap());
    assert_eq!(queue::list(&path).unwrap().len(), 1);
}

fn legacy_without_policy_capability(path: &Path) -> rusqlite::Connection {
    legacy_obligation(path);
    let db = open_initialized(path).unwrap();
    let no_delete: String = db
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='cdr_capability_no_delete'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch(
        "DROP TRIGGER cdr_capability_no_delete;
         DELETE FROM cdr_runtime_capability_requirements WHERE component='async_recovery_policy';
         DROP TABLE cdr_async_recovery_policies;",
    )
    .unwrap();
    db.execute_batch(&no_delete).unwrap();
    db
}

fn schema_entries(db: &rusqlite::Connection) -> Vec<(String, Option<String>)> {
    db.prepare("SELECT name,sql FROM sqlite_schema ORDER BY name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

#[test]
fn extension_upgrade_rejects_ignored_required_capability_and_rolls_back() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    let db = legacy_without_policy_capability(&path);
    db.execute_batch(
        "CREATE TRIGGER ignore_new_policy_capability
         BEFORE INSERT ON cdr_runtime_capability_requirements
         WHEN NEW.component='async_recovery_policy'
         BEGIN SELECT RAISE(IGNORE); END;",
    )
    .unwrap();
    let catalog = schema_entries(&db);
    let evidence =
        serde_json::to_value(async_resolution::inspect(&path, INCIDENT).unwrap()).unwrap();
    drop(db);
    assert!(
        open_initialized(&path).is_err(),
        "migration reported success although its required compatibility marker was not stored"
    );
    // Do not initialize again while inspecting the failed upgrade's original DB.
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        schema_entries(&db),
        catalog,
        "partial schema migration did not roll back"
    );
    let capability:i64=db.query_row(
        "SELECT COUNT(*) FROM cdr_runtime_capability_requirements WHERE component='async_recovery_policy'",
        [],|r|r.get(0),
    ).unwrap();
    assert_eq!(capability, 0);
    assert_eq!(
        serde_json::to_value(async_resolution::inspect(&path, INCIDENT).unwrap()).unwrap(),
        evidence
    );
}

#[test]
fn extension_upgrade_preserves_higher_required_capability_without_authorizing_policy() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    let db = legacy_without_policy_capability(&path);
    db.execute(
        "INSERT INTO cdr_runtime_capability_requirements VALUES('async_recovery_policy',2)",
        [],
    )
    .unwrap();
    drop(db);
    let db = open_initialized(&path).unwrap();
    let version:i64=db.query_row(
        "SELECT format_version FROM cdr_runtime_capability_requirements WHERE component='async_recovery_policy'",
        [],|r|r.get(0),
    ).unwrap();
    assert_eq!(
        version, 2,
        "migration must not downgrade a future requirement"
    );
    assert!(async_resolution::install_reviewed_policy(&path).is_err());
    assert!(!async_resolution::reviewed_policy_installed_in(&db).unwrap());
    assert!(async_resolution::admission_held(&path, INCIDENT).unwrap());
}
