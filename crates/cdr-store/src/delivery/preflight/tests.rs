use super::*;
use crate::schema::checked_read::test_support::{Boundary, HookGuard};
use crate::{StoreError, schema};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug)]
enum Barrier {
    Ready,
    New,
    FirstReply,
    Commentary,
    Goal,
}

const CASES: [Barrier; 5] = [
    Barrier::Ready,
    Barrier::New,
    Barrier::FirstReply,
    Barrier::Commentary,
    Barrier::Goal,
];

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, StoredDelivery) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    crate::mapping::upsert_thread(&path, "thread", "project", "title", 100, 42, 1.0).unwrap();
    let db = schema::open_initialized(&path).unwrap();
    db.execute_batch(
        "INSERT INTO codex_delivery_outbox
        (delivery_id,job_id,target_thread_id,turn_id,channel_id,content,created_at,updated_at)
        VALUES ('job','job','thread','turn',42,'Saved answer',1,1)",
    )
    .unwrap();
    let pending = crate::delivery::select(&db, "job").unwrap();
    (temp, path, pending)
}

fn seed_ingress(db: &Connection) {
    db.execute_batch("INSERT OR IGNORE INTO discord_ingress_journal
        (ingress_id,kind,event_id,channel_id,owner_user_id,payload_json,state,phase,
        target_thread_id,owner_kind,owner_id,confirmation_delivered,created_at,updated_at)
        VALUES ('message:11','message',11,42,3,'{}','owned','durable_prompt','thread','prompt','job',0,1,1)").unwrap();
}

fn seed_new(db: &Connection) {
    seed_ingress(db);
    let identity = crate::new_reply::Identity {
        ingress_id: "message:11".into(),
        job_id: "job".into(),
        thread_id: "thread".into(),
        cwd: "C:/fixture".into(),
        state_db: "C:/fixture/state.sqlite".into(),
        channel_id: 42,
        origin_channel_id: 42,
        event_id: Some(11),
        kind: crate::ingress::IngressKind::Message,
        creation_generation: 1,
        prompt_sha256: crate::final_recovery::sha256("input"),
        acknowledgement: "Ready".into(),
    };
    let outcome = serde_json::json!({
        "new_creation":{"version":1,"cwd":identity.cwd},
        "new_verification":{"thread_id":identity.thread_id,"channel_id":42,
            "prompt_sha256":identity.prompt_sha256}
    });
    db.execute(
        "UPDATE discord_ingress_journal SET outcome_json=? WHERE ingress_id='message:11'",
        [outcome.to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO codex_new_first_replies
        (job_id,ingress_id,identity_json,turn_id,accepted_at,last_error)
        VALUES ('job','message:11',?,'turn',1,'fixture pending')",
        [serde_json::to_string(&identity).unwrap()],
    )
    .unwrap();
}

fn seed(path: &Path, barrier: Barrier) {
    let db = schema::open_initialized(path).unwrap();
    match barrier {
        Barrier::Ready => {}
        Barrier::New => seed_new(&db),
        Barrier::FirstReply => seed_ingress(&db),
        Barrier::Commentary => {
            db.execute_batch(
                "INSERT INTO codex_commentary_outbox
            (delivery_key,job_id,target_thread_id,turn_id,channel_id,text)
            VALUES ('progress','job','thread','turn',42,'Earlier commentary')",
            )
            .unwrap();
        }
        Barrier::Goal => {
            db.execute_batch(
                "INSERT INTO codex_goal_progress
            (thread,turn,channel,content,job_id)
            VALUES ('thread','prior-turn',42,'Earlier legacy goal progress',NULL)",
            )
            .unwrap();
        }
    }
}

fn expected(barrier: Barrier) -> FinalReadiness {
    match barrier {
        Barrier::Ready => FinalReadiness::Ready,
        Barrier::New => FinalReadiness::Held(
            "new first input verification is pending; output remains saved: fixture pending".into(),
        ),
        Barrier::FirstReply => FinalReadiness::FirstReply("message:11".into()),
        Barrier::Commentary => FinalReadiness::Commentary,
        Barrier::Goal => FinalReadiness::GoalProgress,
    }
}

// Frozen pre-change wrapper order: this oracle does not call the new read helper.
fn legacy(path: &Path, pending: &StoredDelivery) -> Result<FinalReadiness> {
    let grant = crate::final_recovery::authorized(path, pending)?;
    if let Some(reason) = crate::new_reply::output_hold(path, &pending.job_id)? {
        return Ok(FinalReadiness::Held(reason));
    }
    if !grant {
        if let Some(id) = crate::first_reply::pending(path, &pending.job_id)? {
            return Ok(FinalReadiness::FirstReply(id));
        }
        if crate::commentary_outbox::has_pending(path, &pending.job_id, None)? {
            return Ok(FinalReadiness::Commentary);
        }
    }
    if crate::goal_progress::has_pending_job(path, &pending.job_id, &pending.target_thread_id)? {
        return Ok(FinalReadiness::GoalProgress);
    }
    Ok(FinalReadiness::Ready)
}

fn grant(path: &Path, pending: &StoredDelivery) -> StoredDelivery {
    seed_ingress(&Connection::open(path).unwrap());
    let key = serde_json::json!([42, "message/error/v1", "inbound-message/11/error-report", 0])
        .to_string();
    crate::delivery_receipt::begin(path, &key, "error-hash").unwrap();
    crate::delivery_receipt::confirm(path, &key, "123").unwrap();
    crate::final_recovery::authorize(
        path,
        &crate::final_recovery::Request {
            delivery_id: pending.delivery_id.clone(),
            job_id: pending.job_id.clone(),
            thread_id: pending.target_thread_id.clone(),
            turn_id: pending.turn_id.clone(),
            channel_id: pending.channel_id,
            original_sha256: crate::final_recovery::sha256(&pending.content),
            ingress_id: "message:11".into(),
            error_receipt_key: key,
            error_message_id: "123".into(),
            error_sha256: "error-hash".into(),
        },
        |content| vec![content.to_owned()],
    )
    .unwrap();
    crate::delivery::list_pending(path).unwrap().remove(0)
}

#[test]
fn every_final_preflight_result_matches_old_wrappers_with_one_read_only_snapshot() {
    for barrier in CASES {
        let (_temp, path, pending) = fixture();
        seed(&path, barrier);
        assert_eq!(legacy(&path, &pending).unwrap(), expected(barrier));
        let boundaries = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&boundaries);
        let hook = HookGuard::install(move |point, db| {
            observed.lock().unwrap().push(point);
            if point == Boundary::Opened {
                assert_eq!(
                    db.pragma_query_value(None, "query_only", |r| r.get::<_, i64>(0))?,
                    1
                );
                assert!(db.execute("DELETE FROM codex_delivery_outbox", []).is_err());
            }
            Ok(())
        });
        assert_eq!(final_preflight(&path, &pending).unwrap(), expected(barrier));
        drop(hook);
        assert_eq!(
            *boundaries.lock().unwrap(),
            [Boundary::Opened, Boundary::Catalog, Boundary::BeforeCommit]
        );
        assert_eq!(crate::delivery::list_pending(&path).unwrap(), [pending]);
    }
}

#[test]
fn final_preflight_preserves_first_reply_commentary_and_legacy_goal_order() {
    let (_temp, path, pending) = fixture();
    for barrier in [Barrier::FirstReply, Barrier::Commentary, Barrier::Goal] {
        seed(&path, barrier);
    }
    let db = Connection::open(&path).unwrap();
    assert_eq!(
        final_preflight(&path, &pending).unwrap(),
        expected(Barrier::FirstReply)
    );
    db.execute(
        "UPDATE discord_ingress_journal SET confirmation_delivered=1",
        [],
    )
    .unwrap();
    assert_eq!(
        final_preflight(&path, &pending).unwrap(),
        FinalReadiness::Commentary
    );
    db.execute("DELETE FROM codex_commentary_outbox", [])
        .unwrap();
    assert_eq!(
        final_preflight(&path, &pending).unwrap(),
        FinalReadiness::GoalProgress
    );
    db.execute("DELETE FROM codex_goal_progress", []).unwrap();
    assert_eq!(
        final_preflight(&path, &pending).unwrap(),
        FinalReadiness::Ready
    );
    assert_eq!(crate::delivery::list_pending(&path).unwrap(), [pending]);
}

#[test]
fn predicate_short_circuit_does_not_execute_a_later_failing_query() {
    for barrier in [Barrier::New, Barrier::FirstReply, Barrier::Commentary] {
        let (_temp, path, pending) = fixture();
        seed(&path, barrier);
        let db = Connection::open(&path).unwrap();
        // Inject at the predicate layer, after schema validation's scope.
        // Full final_preflight must instead reject this incomplete schema.
        db.execute_batch("DROP TABLE codex_goal_progress").unwrap();
        assert_eq!(read(&db, &pending).unwrap(), expected(barrier));
        db.execute_batch(
            "DELETE FROM codex_new_first_replies;
            DELETE FROM discord_ingress_journal; DELETE FROM codex_commentary_outbox",
        )
        .unwrap();
        assert!(
            read(&db, &pending).is_err(),
            "later query must genuinely fail"
        );
    }
}

#[test]
fn invalid_grant_precedes_new_hold_and_preserves_original_error() {
    let (_temp, path, pending) = fixture();
    seed(&path, Barrier::New);
    let pending = grant(&path, &pending);
    assert_eq!(legacy(&path, &pending).unwrap(), expected(Barrier::New));
    assert_eq!(
        final_preflight(&path, &pending).unwrap(),
        expected(Barrier::New)
    );
    Connection::open(&path)
        .unwrap()
        .execute("UPDATE discord_ingress_journal SET owner_user_id=4", [])
        .unwrap();
    assert!(
        crate::new_reply::output_hold(&path, "job")
            .unwrap()
            .is_some()
    );
    let old = legacy(&path, &pending).unwrap_err();
    let new = final_preflight(&path, &pending).unwrap_err();
    assert!(matches!(&old, StoreError::Integrity(_)));
    assert!(matches!(&new, StoreError::Integrity(_)));
    assert_eq!(new.to_string(), old.to_string());
}

#[test]
fn granted_final_still_validates_earlier_progress_inside_the_grant() {
    for barrier in [Barrier::Commentary, Barrier::Goal] {
        let (_temp, path, pending) = fixture();
        let pending = grant(&path, &pending);
        assert_eq!(
            final_preflight(&path, &pending).unwrap(),
            FinalReadiness::Ready
        );
        seed(&path, barrier);
        let old = legacy(&path, &pending).unwrap_err();
        let new = final_preflight(&path, &pending).unwrap_err();
        assert!(matches!(&new, StoreError::Integrity(_)));
        assert_eq!(new.to_string(), old.to_string());
        assert!(
            new.to_string()
                .contains("saved final held behind undelivered progress")
        );
    }
}

#[test]
fn final_preflight_finish_failure_never_publishes_any_readiness() {
    for barrier in CASES {
        let (_temp, path, pending) = fixture();
        seed(&path, barrier);
        let hook = HookGuard::install(|point, _| {
            if point == Boundary::BeforeCommit {
                return Err(StoreError::Integrity(
                    "injected final preflight finish failure".into(),
                ));
            }
            Ok(())
        });
        assert!(
            final_preflight(&path, &pending)
                .unwrap_err()
                .to_string()
                .contains("injected final preflight finish failure")
        );
        drop(hook);
        assert_eq!(crate::delivery::list_pending(&path).unwrap(), [pending]);
    }
}

#[test]
fn final_preflight_rejects_an_ended_snapshot() {
    let (_temp, path, pending) = fixture();
    let hook = HookGuard::install(|point, db| {
        if point == Boundary::Catalog {
            db.execute_batch("ROLLBACK")?;
        }
        Ok(())
    });
    assert!(final_preflight(&path, &pending).is_err());
    drop(hook);
    assert_eq!(crate::delivery::list_pending(&path).unwrap(), [pending]);
}

#[test]
fn final_preflight_never_creates_or_repairs_an_unknown_database() {
    let (temp, path, pending) = fixture();
    let missing = temp.path().join("missing.sqlite");
    assert!(final_preflight(&missing, &pending).is_err());
    assert!(!missing.exists());
    let db = Connection::open(&path).unwrap();
    for version in [0, schema::LATEST_STORE_SCHEMA_VERSION + 1] {
        db.pragma_update(None, "user_version", version).unwrap();
        assert!(matches!(final_preflight(&path, &pending),
            Err(StoreError::UnsupportedVersion { found, .. }) if found == version));
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            version
        );
    }
}

#[test]
fn final_preflight_does_not_repair_a_missing_required_table() {
    let (_temp, path, pending) = fixture();
    let db = Connection::open(&path).unwrap();
    db.execute_batch("DROP TABLE codex_goal_progress").unwrap();
    assert!(final_preflight(&path, &pending).is_err());
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name='codex_goal_progress'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        schema::LATEST_STORE_SCHEMA_VERSION
    );
    assert_eq!(crate::delivery::select(&db, "job").unwrap(), pending);
}
