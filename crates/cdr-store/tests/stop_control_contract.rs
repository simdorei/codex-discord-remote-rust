use cdr_store::{
    ingress::{
        self, IngressKind, NewIngress,
        stop::{StopScope, control},
    },
    queue::{NewQueueJob, begin_attempt, enqueue, list_filtered, mark_running},
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::path::PathBuf;

struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("store.sqlite");
        let f = Self { _temp: temp, db };
        f.add("original", 101);
        begin_attempt(&f.db, "original", &[], 7).unwrap();
        mark_running(&f.db, "original", "owned-a", 7).unwrap();
        f
    }
    fn add(&self, id: &str, event: i64) {
        enqueue(
            &self.db,
            NewQueueJob {
                job_id: id,
                target_thread_id: "thread-a",
                channel_id: 99,
                owner_user_id: Some(20),
                discord_message_id: Some(event),
                app_server_generation: 7,
                prompt: "original input",
                queued: true,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
    }
    fn accept(&self) -> control::StopControl {
        control::accept_running(
            &self.db,
            scope(),
            &binding(),
            None,
            ("resident-a", 7),
            || Ok(()),
        )
        .unwrap()
        .unwrap()
    }
    fn terminal(&self, resident: &str) {
        cdr_store::observed_completion::record_for_resident(
            &self.db,
            "thread-a",
            "owned-a",
            7,
            r#"{"threadId":"thread-a","turn":{"id":"owned-a","status":"interrupted"}}"#,
            resident,
        )
        .unwrap();
    }
}
fn scope() -> StopScope<'static> {
    StopScope {
        target: "thread-a",
        channel: 99,
        owner: 20,
    }
}
fn binding() -> Value {
    json!({"target":"thread-a","route":"Explicit","command":{"Stop":{"reference":"thread-a"}}})
}
fn params_for_turn() -> Value {
    json!({"threadId":"thread-a","turnId":"owned-a"})
}

#[test]
fn running_receipt_survives_result_confirmation_and_prior_hold_is_immutable() {
    let f = Fixture::new();
    let db = Connection::open(&f.db).unwrap();
    db.execute(
        "INSERT INTO cdr_execution_holds VALUES('original','thread-a','prior','prior evidence',1)",
        [],
    )
    .unwrap();
    let frozen = binding();
    ingress::admit(&f.db,&NewIngress {
        ingress_id:"message:500".into(),kind:IngressKind::Message,event_id:Some(500),
        application_id:None,channel_id:99,owner_user_id:20,source_message_id:Some(500),
        payload:json!({"version":1,"plan":{"Execute":frozen["command"]},"lifecycle_binding":frozen}),
        target_thread_id:Some("thread-a".into()),canonical_owner:None,now:2.0,
    }).unwrap();
    assert!(
        ingress::begin_execution(&f.db, "message:500", "processing", Some("thread-a"), 3.0)
            .unwrap()
    );
    let original =
        serde_json::to_value(list_filtered(&f.db, Some("thread-a"), None).unwrap()).unwrap();
    let expected = ingress::get(&f.db, "message:500").unwrap().unwrap();
    let receipt = control::accept_running(
        &f.db,
        scope(),
        &frozen,
        Some(&expected),
        ("resident-a", 7),
        || Ok(()),
    )
    .unwrap()
    .unwrap();
    ingress::record_result(
        &f.db,
        "message:500",
        &json!({"response":"accepted","waits_for_final":false}),
        4.0,
    )
    .unwrap();
    ingress::confirm(&f.db, "message:500", 5.0).unwrap();
    let saved = ingress::get(&f.db, "message:500").unwrap().unwrap();
    assert_eq!(
        saved.outcome.as_ref().unwrap()["stop_receipt"]["jobs"],
        json!(["original"])
    );
    assert_eq!(
        saved.outcome.as_ref().unwrap()["stop_receipt"]["execution_end_confirmed"],
        false
    );
    assert_eq!(receipt.operation_id, "stop:message:500");
    assert_eq!(
        control::phase(&f.db, &receipt.operation_id)
            .unwrap()
            .as_deref(),
        Some("accepted")
    );
    assert_eq!(
        serde_json::to_value(list_filtered(&f.db, Some("thread-a"), None).unwrap()).unwrap(),
        original
    );
    let prior: (String, String) = db
        .query_row(
            "SELECT reason,evidence_json FROM cdr_execution_holds WHERE job_id='original'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(prior, ("prior".into(), "prior evidence".into()));
}

#[test]
fn claim_and_wire_are_one_use_across_reopen_ack_is_not_end() {
    let f = Fixture::new();
    let original = f.accept();
    let claim = control::claim(&f.db, &original, || Ok(()))
        .unwrap()
        .unwrap();
    assert!(control::pending_after(&f.db, 0).unwrap().is_empty());
    assert!(
        control::claim(&f.db, &original, || Ok(()))
            .unwrap()
            .is_none()
    );
    let value = serde_json::to_value(&claim).unwrap();
    control::begin_wire(
        &f.db,
        &value,
        ("resident-a", 7),
        ("wire-attempt", "1"),
        &params_for_turn(),
    )
    .unwrap();
    assert!(
        control::begin_wire(
            &f.db,
            &value,
            ("resident-a", 7),
            ("second", "2"),
            &params_for_turn()
        )
        .is_err()
    );
    assert!(
        control::begin_wire(
            &f.db,
            &value,
            ("new-resident", 7),
            ("second", "2"),
            &params_for_turn()
        )
        .is_err()
    );
    control::finish_wire(
        &f.db,
        &value,
        ("resident-a", 7),
        ("wire-attempt", "1"),
        "reply_ok",
    )
    .unwrap();
    assert_eq!(
        control::phase(&f.db, &original.operation_id)
            .unwrap()
            .as_deref(),
        Some("acknowledged")
    );
    assert!(control::target_is_held(&f.db, "thread-a").unwrap());
    assert!(control::pending_after(&f.db, 0).unwrap().is_empty());
}

#[test]
fn exact_terminal_settles_but_other_resident_and_starting_uncertainty_do_not() {
    for unresolved_starting in [false, true] {
        let f = Fixture::new();
        f.add("pending", 102);
        if unresolved_starting {
            begin_attempt(&f.db, "pending", &[], 7).unwrap();
        }
        let original = f.accept();
        f.terminal("wrong-resident");
        assert!(control::target_is_held(&f.db, "thread-a").unwrap());
        f.terminal("resident-a");
        assert_eq!(
            control::target_is_held(&f.db, "thread-a").unwrap(),
            unresolved_starting
        );
        assert_eq!(
            control::phase(&f.db, &original.operation_id)
                .unwrap()
                .as_deref(),
            Some(if unresolved_starting {
                "unknown"
            } else {
                "settled"
            })
        );
        assert!(
            cdr_store::execution_hold::reason(&f.db, "original")
                .unwrap()
                .is_some()
        );
        assert!(
            cdr_store::execution_hold::reason(&f.db, "pending")
                .unwrap()
                .is_some()
        );
        assert_eq!(
            list_filtered(&f.db, Some("thread-a"), None).unwrap().len(),
            2
        );
    }
}

#[test]
fn terminal_before_final_wire_claim_revokes_interrupt() {
    let f = Fixture::new();
    let original = f.accept();
    let claim = control::claim(&f.db, &original, || Ok(()))
        .unwrap()
        .unwrap();
    f.terminal("resident-a");
    assert!(
        control::begin_wire(
            &f.db,
            &serde_json::to_value(claim).unwrap(),
            ("resident-a", 7),
            ("attempt", "1"),
            &params_for_turn()
        )
        .is_err()
    );
    assert_eq!(
        control::phase(&f.db, &original.operation_id)
            .unwrap()
            .as_deref(),
        Some("settled")
    );
}

#[test]
fn receipt_write_failure_rolls_back_holds_and_queue_is_unchanged() {
    for table in ["cdr_execution_holds", "cdr_stop_controls"] {
        let f = Fixture::new();
        let before =
            serde_json::to_value(list_filtered(&f.db, Some("thread-a"), None).unwrap()).unwrap();
        Connection::open(&f.db).unwrap().execute_batch(&format!(
            "CREATE TRIGGER reject_stop BEFORE INSERT ON {table} BEGIN SELECT RAISE(IGNORE); END;"
        )).unwrap();
        assert!(
            control::accept_running(&f.db, scope(), &binding(), None, ("resident-a", 7), || Ok(
                ()
            ))
            .is_err()
        );
        assert!(control::pending_after(&f.db, 0).unwrap().is_empty());
        assert!(
            cdr_store::execution_hold::reason(&f.db, "original")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            serde_json::to_value(list_filtered(&f.db, Some("thread-a"), None).unwrap()).unwrap(),
            before
        );
    }
}

#[test]
fn remap_after_worker_claim_blocks_final_wire_without_borrowing_new_target() {
    let f = Fixture::new();
    let db = Connection::open(&f.db).unwrap();
    db.execute(
        "INSERT INTO mirror_threads VALUES('thread-a','project','A',98,99,1)",
        [],
    )
    .unwrap();
    let mapped =
        json!({"target":"thread-a","route":"Mapped","command":{"Stop":{"reference":null}}});
    let original =
        control::accept_running(&f.db, scope(), &mapped, None, ("resident-a", 7), || Ok(()))
            .unwrap()
            .unwrap();
    let claim = control::claim(&f.db, &original, || Ok(()))
        .unwrap()
        .unwrap();
    db.execute(
        "UPDATE mirror_threads SET codex_thread_id='thread-b' WHERE codex_thread_id='thread-a'",
        [],
    )
    .unwrap();
    assert!(
        control::begin_wire(
            &f.db,
            &serde_json::to_value(claim).unwrap(),
            ("resident-a", 7),
            ("attempt", "1"),
            &params_for_turn()
        )
        .is_err()
    );
    assert!(control::target_is_held(&f.db, "thread-a").unwrap());
    assert!(!control::target_is_held(&f.db, "thread-b").unwrap());
}

#[test]
fn repeated_stop_cannot_grant_second_interrupt_of_original_turn() {
    let f = Fixture::new();
    let first = f.accept();
    let claim = control::claim(&f.db, &first, || Ok(())).unwrap().unwrap();
    control::record_error(&f.db, &claim, "outcome unknown").unwrap();
    let second = f.accept();
    assert_ne!(first.operation_id, second.operation_id);
    assert!(control::claim(&f.db, &second, || Ok(())).unwrap().is_none());
    assert!(control::target_is_held(&f.db, "thread-a").unwrap());
}

#[test]
fn extension_migration_backs_up_and_preserves_original_queue() {
    let f = Fixture::new();
    let before =
        serde_json::to_value(list_filtered(&f.db, Some("thread-a"), None).unwrap()).unwrap();
    let mut db = Connection::open(&f.db).unwrap();
    db.execute_batch("DROP TABLE cdr_stop_controls;").unwrap();
    let backup = cdr_store::schema::initialize(&mut db, &f.db).unwrap();
    assert!(backup.is_some());
    assert_eq!(
        serde_json::to_value(list_filtered(&f.db, Some("thread-a"), None).unwrap()).unwrap(),
        before
    );
    let columns: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info(?)",
            params!["cdr_stop_controls"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(columns, 13);
}
