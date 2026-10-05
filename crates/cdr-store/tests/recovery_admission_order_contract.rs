use cdr_store::{
    ingress::{self, IngressKind, NewIngress},
    processed, schema,
};
use rusqlite::{Connection, params};
use serde_json::json;
use std::path::Path;

fn request(id: i64, now: f64) -> NewIngress {
    NewIngress {
        ingress_id: format!("message:{id}"),
        kind: IngressKind::Message,
        event_id: Some(id),
        application_id: Some(99),
        channel_id: 10,
        owner_user_id: 20,
        source_message_id: Some(id),
        payload: json!({"version":1,"content":"private input","plan":{"Execute":{"Ask":{"prompt":"private input"}}},"processing_mode":"normal"}),
        target_thread_id: Some("ordinary-a".into()),
        canonical_owner: None,
        now,
    }
}

fn feature(db: &Connection) {
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='cdr_recovery_ingress_order')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        exists,
        "R4-D3A: first durable admission has no persistent ordinal"
    );
}

fn ordinal(db: &Connection, id: &str) -> (i64, String, Option<String>) {
    db.query_row(
        "SELECT sequence,origin,identity_sha256 FROM cdr_recovery_ingress_order WHERE ingress_id=?",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .unwrap()
}

fn count(db: &Connection) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM cdr_recovery_ingress_order",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

fn absent(path: &Path, db: &Connection, id: i64) {
    assert!(
        ingress::get(path, &format!("message:{id}"))
            .unwrap()
            .is_none()
    );
    assert!(!processed::is_processed(path, id).unwrap());
    assert_eq!(count(db), 0);
    assert!(cdr_store::queue::list(path).unwrap().is_empty());
}

#[test]
fn first_admission_order_ignores_wall_clock_and_duplicate_payload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite");
    let db = schema::open_initialized(&path).unwrap();
    feature(&db);
    for (id, time) in [(501, 100.0), (502, 100.0), (503, 1.0)] {
        assert!(ingress::admit(&path, &request(id, time)).unwrap().created);
    }
    let first = ordinal(&db, "message:501");
    let second = ordinal(&db, "message:502");
    let third = ordinal(&db, "message:503");
    assert!(first.0 > 0 && first.0 < second.0 && second.0 < third.0);
    assert_eq!(first.1, "admitted");
    assert_eq!(first.2.as_ref().unwrap().len(), 64);
    let mut repeated = request(501, 1000.0);
    repeated.ingress_id = "message:alias".into();
    repeated.payload = json!({"content":"replacement is not a new admission"});
    assert!(!ingress::admit(&path, &repeated).unwrap().created);
    assert_eq!(ordinal(&db, "message:501"), first);
    assert_eq!(count(&db), 3);
    repeated.owner_user_id = 21;
    assert!(ingress::admit(&path, &repeated).is_err());
    assert_eq!(count(&db), 3);
}

#[test]
fn order_write_abort_ignore_and_postwrite_identity_change_roll_back_custody() {
    for (timing, body) in [
        ("BEFORE", "SELECT RAISE(ABORT,'injected order failure');"),
        ("BEFORE", "SELECT RAISE(IGNORE);"),
        (
            "AFTER",
            "UPDATE discord_ingress_journal SET owner_user_id=21 WHERE ingress_id=NEW.ingress_id;",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private.sqlite");
        let db = schema::open_initialized(&path).unwrap();
        feature(&db);
        db.execute_batch(&format!(
            "CREATE TRIGGER fixture_order_fault {timing} INSERT ON cdr_recovery_ingress_order BEGIN {body} END;"
        )).unwrap();
        assert!(
            ingress::admit(&path, &request(510, 100.0)).is_err(),
            "{body}"
        );
        absent(&path, &db, 510);
    }
}

#[test]
fn processed_claim_failure_rolls_back_the_new_ordinal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite");
    let db = schema::open_initialized(&path).unwrap();
    feature(&db);
    db.execute_batch(
        "CREATE TRIGGER fixture_claim_fault BEFORE INSERT ON discord_processed_messages
         BEGIN SELECT RAISE(ABORT,'injected processed failure'); END;",
    )
    .unwrap();
    assert!(ingress::admit(&path, &request(511, 100.0)).is_err());
    absent(&path, &db, 511);
}

#[test]
fn ordinal_is_immutable_and_an_erased_journal_cannot_requalify_its_event() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite");
    let db = schema::open_initialized(&path).unwrap();
    feature(&db);
    ingress::admit(&path, &request(520, 100.0)).unwrap();
    let original = ordinal(&db, "message:520");
    for sql in [
        "UPDATE cdr_recovery_ingress_order SET sequence=999",
        "DELETE FROM cdr_recovery_ingress_order",
        "INSERT OR REPLACE INTO cdr_recovery_ingress_order
         (sequence,ingress_id,kind,event_id,origin,identity_sha256)
         SELECT sequence,ingress_id,kind,event_id,origin,identity_sha256
         FROM cdr_recovery_ingress_order",
    ] {
        assert!(db.execute_batch(sql).is_err(), "{sql}");
        assert_eq!(ordinal(&db, "message:520"), original);
    }
    db.execute(
        "DELETE FROM discord_ingress_journal WHERE ingress_id='message:520'",
        [],
    )
    .unwrap();
    db.execute(
        "DELETE FROM discord_processed_messages WHERE message_id=520",
        [],
    )
    .unwrap();
    assert!(ingress::admit(&path, &request(520, 200.0)).is_err());
    assert_eq!(ordinal(&db, "message:520"), original);
    assert!(ingress::get(&path, "message:520").unwrap().is_none());
    assert!(!processed::is_processed(&path, 520).unwrap());
}

fn remove_order_feature_for_legacy_fixture(db: &Connection) {
    let capability_guard: String = db
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='cdr_capability_no_delete'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    db.execute_batch(
        "DROP TRIGGER cdr_recovery_ingress_order_no_update;
         DROP TRIGGER cdr_recovery_ingress_order_no_delete;
         DROP TRIGGER cdr_recovery_ingress_order_no_replace;
         DROP TABLE cdr_recovery_ingress_order;
         DROP TRIGGER cdr_capability_no_delete;
         DELETE FROM cdr_runtime_capability_requirements WHERE component='recovery_admission_order';"
    ).unwrap();
    db.execute_batch(&capability_guard).unwrap();
}

#[test]
fn legacy_journal_and_id_only_claims_never_become_fresh_admissions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite");
    let db = schema::open_initialized(&path).unwrap();
    feature(&db);
    ingress::admit(&path, &request(530, 100.0)).unwrap();
    assert!(processed::claim(&path, 531, 90.0).unwrap());
    let saved = ingress::get(&path, "message:530").unwrap().unwrap();
    remove_order_feature_for_legacy_fixture(&db);
    drop(db);
    let db = schema::open_initialized(&path).unwrap();
    assert_eq!(ingress::get(&path, "message:530").unwrap().unwrap(), saved);
    let old = ordinal(&db, "message:530");
    assert_eq!(old.1, "legacy");
    assert!(old.2.is_none());
    let id_only: (Option<String>, String, Option<String>) = db
        .query_row(
            "SELECT ingress_id,origin,identity_sha256 FROM cdr_recovery_ingress_order
         WHERE kind='message' AND event_id=531",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(id_only, (None, "legacy".into(), None));
    let boundary: i64 = db
        .query_row(
            "SELECT MAX(sequence) FROM cdr_recovery_ingress_order",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!ingress::admit(&path, &request(530, 200.0)).unwrap().created);
    assert!(!ingress::admit(&path, &request(531, 200.0)).unwrap().created);
    assert!(ingress::admit(&path, &request(532, 1.0)).unwrap().created);
    assert!(ordinal(&db, "message:532").0 > boundary);
    assert_eq!(ordinal(&db, "message:532").1, "admitted");
    assert_eq!(count(&db), 3);
}

#[test]
fn partial_history_or_unsupported_capability_is_not_recreated() {
    for fault in [
        "DROP TRIGGER cdr_recovery_ingress_order_no_delete",
        "DROP TABLE cdr_recovery_ingress_order",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private.sqlite");
        let db = schema::open_initialized(&path).unwrap();
        feature(&db);
        ingress::admit(&path, &request(540, 100.0)).unwrap();
        db.execute_batch(fault).unwrap();
        let schema_before: String = db
            .query_row(
                "SELECT group_concat(type||':'||name||':'||COALESCE(sql,''),char(10))
             FROM (SELECT type,name,sql FROM sqlite_schema ORDER BY name)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(schema::open_initialized(&path).is_err(), "{fault}");
        let schema_after: String = db
            .query_row(
                "SELECT group_concat(type||':'||name||':'||COALESCE(sql,''),char(10))
             FROM (SELECT type,name,sql FROM sqlite_schema ORDER BY name)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(schema_after, schema_before);
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite");
    let db = schema::open_initialized(&path).unwrap();
    feature(&db);
    // The cache proves structure, not mutable capability values. Exercise the
    // actual admission boundary with an already warm structural cache.
    drop(schema::open_initialized(&path).unwrap());
    db.execute(
        "UPDATE cdr_runtime_capability_requirements SET format_version=2
         WHERE component='recovery_admission_order'",
        [],
    )
    .unwrap();
    assert!(ingress::admit(&path, &request(541, 100.0)).is_err());
    absent(&path, &db, 541);
    let required: i64 = db
        .query_row(
            "SELECT format_version FROM cdr_runtime_capability_requirements
         WHERE component='recovery_admission_order'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        required, 2,
        "refusal must not downgrade the required version"
    );
}

#[test]
fn capability_change_after_ordinal_insert_rolls_back_admission() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite");
    let db = schema::open_initialized(&path).unwrap();
    feature(&db);
    db.execute_batch(
        "CREATE TRIGGER fixture_order_version AFTER INSERT ON cdr_recovery_ingress_order
         BEGIN UPDATE cdr_runtime_capability_requirements SET format_version=2
         WHERE component='recovery_admission_order'; END;",
    )
    .unwrap();
    assert!(ingress::admit(&path, &request(542, 100.0)).is_err());
    absent(&path, &db, 542);
    let required: i64 = db
        .query_row(
            "SELECT format_version FROM cdr_runtime_capability_requirements
         WHERE component='recovery_admission_order'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        required, 1,
        "the failed admission must roll back its trigger too"
    );
}

#[test]
fn concurrent_admissions_use_one_persisted_order_and_restart_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite");
    let db = schema::open_initialized(&path).unwrap();
    feature(&db);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let workers: Vec<_> = [550, 551]
        .into_iter()
        .map(|id| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                ingress::admit(&path, &request(id, 100.0)).unwrap().created
            })
        })
        .collect();
    barrier.wait();
    for worker in workers {
        assert!(worker.join().unwrap());
    }
    let first = ordinal(&db, "message:550");
    let second = ordinal(&db, "message:551");
    assert_ne!(first.0, second.0);
    assert!(first.0 > 0 && second.0 > 0);
    drop(db);
    let db = schema::open_initialized(&path).unwrap();
    assert_eq!(ordinal(&db, "message:550"), first);
    assert_eq!(ordinal(&db, "message:551"), second);
    ingress::admit(&path, &request(552, 0.0)).unwrap();
    assert!(ordinal(&db, "message:552").0 > first.0.max(second.0));
    assert_eq!(count(&db), 3);
}

#[test]
fn exhausted_sequence_refuses_admission_without_partial_custody() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite");
    let db = schema::open_initialized(&path).unwrap();
    feature(&db);
    db.execute(
        "INSERT INTO cdr_recovery_ingress_order
         (sequence,ingress_id,kind,event_id,origin,identity_sha256)
         VALUES(?,'fixture:exhausted','action',NULL,'legacy',NULL)",
        [i64::MAX],
    )
    .unwrap();
    assert!(ingress::admit(&path, &request(560, 100.0)).is_err());
    assert!(ingress::get(&path, "message:560").unwrap().is_none());
    assert!(!processed::is_processed(&path, 560).unwrap());
    assert_eq!(count(&db), 1);
    assert_eq!(ordinal(&db, "fixture:exhausted").0, i64::MAX);
}

#[test]
fn mutable_custody_progress_does_not_rewrite_the_original_identity_digest() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite");
    let db = schema::open_initialized(&path).unwrap();
    feature(&db);
    ingress::admit(&path, &request(570, 100.0)).unwrap();
    let original = ordinal(&db, "message:570");
    assert!(
        ingress::begin_execution(
            &path,
            "message:570",
            "processing",
            Some("ordinary-a"),
            101.0
        )
        .unwrap()
    );
    ingress::record_result(&path, "message:570", &json!({"result":"done"}), 102.0).unwrap();
    assert_eq!(ordinal(&db, "message:570"), original);
    let exposed: i64 = db.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('cdr_recovery_ingress_order') WHERE name LIKE '%payload%' OR name LIKE '%prompt%'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(
        exposed, 0,
        "the ordinal ledger must not duplicate private prompt bodies"
    );
    let canonical: i64 = db.query_row(
        "SELECT COUNT(*) FROM cdr_recovery_ingress_order WHERE ingress_id=? AND event_id=? AND origin='admitted'",
        params!["message:570", 570], |row| row.get(0),
    ).unwrap();
    assert_eq!(canonical, 1);
}

#[test]
fn processed_write_capability_change_rolls_back_admission() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite");
    let db = schema::open_initialized(&path).unwrap();
    feature(&db);
    db.execute_batch(
        "CREATE TRIGGER fixture_processed_version AFTER INSERT ON discord_processed_messages
         BEGIN UPDATE cdr_runtime_capability_requirements SET format_version=2
         WHERE component='recovery_admission_order'; END;",
    )
    .unwrap();
    assert!(ingress::admit(&path, &request(582, 100.0)).is_err());
    absent(&path, &db, 582);
    let required: i64 = db
        .query_row(
            "SELECT format_version FROM cdr_runtime_capability_requirements
         WHERE component='recovery_admission_order'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        required, 1,
        "the last-write capability change must roll back too"
    );
}

#[test]
fn processed_write_identity_change_rolls_back_admission() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite");
    let db = schema::open_initialized(&path).unwrap();
    feature(&db);
    db.execute_batch(
        "CREATE TRIGGER fixture_processed_identity AFTER INSERT ON discord_processed_messages
         BEGIN UPDATE discord_ingress_journal SET owner_user_id=21
         WHERE kind='message' AND event_id=NEW.message_id; END;",
    )
    .unwrap();
    assert!(ingress::admit(&path, &request(583, 100.0)).is_err());
    absent(&path, &db, 583);
}

fn busy_canonical_write_rejects_late_change(body: &str) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite");
    let db = schema::open_initialized(&path).unwrap();
    feature(&db);
    let choice = cdr_store::claims::create_busy_choice(
        &path,
        cdr_store::claims::NewBusyChoice {
            owner_user_id: 20,
            channel_id: 10,
            target_thread_id: Some("ordinary-a"),
            prompt: "private input",
            allow_steer: false,
            now: 90.0,
            time_to_live: 600.0,
        },
    )
    .unwrap();
    let mut first = request(580, 100.0);
    first.kind = IngressKind::Interaction;
    first.ingress_id = "interaction:580".into();
    let admitted = ingress::admit_busy_interaction(&path, &first, &choice, "queue").unwrap();
    assert!(admitted.created);
    let original = admitted.record.unwrap();
    let original_order = ordinal(&db, &first.ingress_id);
    db.execute_batch(&format!(
        "CREATE TRIGGER fixture_busy_late AFTER UPDATE OF state ON discord_ingress_journal
         WHEN NEW.ingress_id='interaction:581' BEGIN {body} END;"
    ))
    .unwrap();
    let mut next = first.clone();
    next.ingress_id = "interaction:581".into();
    next.event_id = Some(581);
    next.source_message_id = Some(581);
    next.now = 101.0;
    assert!(ingress::admit_busy_interaction(&path, &next, &choice, "queue").is_err());
    assert!(ingress::get(&path, &next.ingress_id).unwrap().is_none());
    assert_eq!(
        ingress::get(&path, &first.ingress_id).unwrap().unwrap(),
        original
    );
    assert_eq!(ordinal(&db, &first.ingress_id), original_order);
    assert_eq!(count(&db), 1);
    assert!(cdr_store::queue::list(&path).unwrap().is_empty());
    let required: i64 = db
        .query_row(
            "SELECT format_version FROM cdr_runtime_capability_requirements
         WHERE component='recovery_admission_order'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(required, 1);
    // A private fault-free control: this explicit fixture call is a canonical
    // receipt for the original operation, not new permission to run its prompt.
    db.execute_batch("DROP TRIGGER fixture_busy_late;").unwrap();
    let repeated = ingress::admit_busy_interaction(&path, &next, &choice, "queue").unwrap();
    assert!(!repeated.created && repeated.canonical_repeat_created);
    assert_eq!(repeated.record.unwrap().phase, "canonical_duplicate");
    assert!(ordinal(&db, &next.ingress_id).0 > original_order.0);
    assert_eq!(ordinal(&db, &first.ingress_id), original_order);
    assert_eq!(count(&db), 2);
    assert!(cdr_store::queue::list(&path).unwrap().is_empty());
}

#[test]
fn busy_canonical_write_capability_change_rolls_back_admission() {
    busy_canonical_write_rejects_late_change(
        "UPDATE cdr_runtime_capability_requirements SET format_version=2
         WHERE component='recovery_admission_order';",
    );
}

#[test]
fn busy_canonical_write_identity_change_rolls_back_admission() {
    busy_canonical_write_rejects_late_change(
        "UPDATE discord_ingress_journal SET owner_user_id=21 WHERE ingress_id=NEW.ingress_id;",
    );
}
