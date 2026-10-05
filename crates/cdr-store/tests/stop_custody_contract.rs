use cdr_store::{
    StoreError,
    ingress::{
        self, IngressKind, NewIngress, StoredIngress,
        stop::{StopScope, accept_nonrunning},
    },
    queue::{self, NewQueueJob},
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
    binding: Value,
    record: StoredIngress,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        queue::enqueue(
            &db,
            NewQueueJob {
                job_id: "original",
                target_thread_id: "target",
                channel_id: 99,
                owner_user_id: Some(20),
                discord_message_id: Some(100),
                app_server_generation: 7,
                prompt: "preserved input",
                queued: true,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        let binding = json!({"target":"target","route":"Explicit",
            "command":{"Stop":{"reference":"target"}}});
        ingress::admit(
            &db,
            &NewIngress {
                ingress_id: "message:101".into(),
                kind: IngressKind::Message,
                event_id: Some(101),
                application_id: None,
                channel_id: 99,
                owner_user_id: 20,
                source_message_id: Some(101),
                payload: json!({"version":1,"plan":{"Execute":binding["command"]},
                "lifecycle_binding":binding}),
                target_thread_id: Some("target".into()),
                canonical_owner: None,
                now: 2.0,
            },
        )
        .unwrap();
        assert!(
            ingress::begin_execution(&db, "message:101", "processing", Some("target"), 3.0)
                .unwrap()
        );
        let record = ingress::get(&db, "message:101").unwrap().unwrap();
        Self {
            _temp: temp,
            db,
            binding,
            record,
        }
    }

    fn scope(&self) -> StopScope<'_> {
        StopScope {
            target: self.record.target_thread_id.as_deref().unwrap(),
            channel: self.record.channel_id,
            owner: self.record.owner_user_id,
        }
    }

    fn jobs(&self) -> Value {
        serde_json::to_value(queue::list_filtered(&self.db, None, None).unwrap()).unwrap()
    }

    fn hold_count(&self) -> i64 {
        Connection::open(&self.db)
            .unwrap()
            .query_row("SELECT count(*) FROM cdr_execution_holds", [], |r| r.get(0))
            .unwrap()
    }
}

#[test]
fn original_ingress_is_claimed_once_and_preserves_queue() {
    let f = Fixture::new();
    let before = f.jobs();
    let result = accept_nonrunning(&f.db, f.scope(), &f.binding, Some(&f.record), || Ok(()))
        .unwrap()
        .unwrap();
    assert_eq!(result.jobs, ["original"]);
    assert_eq!(f.jobs(), before);
    let claimed = ingress::get(&f.db, "message:101").unwrap().unwrap();
    assert_eq!(claimed.phase, "stop_accepted");
    assert_eq!(
        claimed.outcome.as_ref().unwrap()["execution_end_confirmed"],
        false
    );
    assert!(accept_nonrunning(&f.db, f.scope(), &f.binding, Some(&f.record), || Ok(())).is_err());
    assert!(accept_nonrunning(&f.db, f.scope(), &f.binding, Some(&claimed), || Ok(())).is_err());
    assert_eq!(f.hold_count(), 1);
}

#[test]
fn different_owner_channel_target_or_command_cannot_accept_stop() {
    let f = Fixture::new();
    let before = f.jobs();
    for scope in [
        StopScope {
            target: "target",
            channel: 99,
            owner: 21,
        },
        StopScope {
            target: "target",
            channel: 98,
            owner: 20,
        },
        StopScope {
            target: "another",
            channel: 99,
            owner: 20,
        },
    ] {
        assert!(accept_nonrunning(&f.db, scope, &f.binding, Some(&f.record), || Ok(())).is_err());
    }
    let mut altered = f.binding.clone();
    altered["command"] = json!({"Recover":{"reference":"target"}});
    assert!(accept_nonrunning(&f.db, f.scope(), &altered, Some(&f.record), || Ok(())).is_err());
    assert_eq!(f.hold_count(), 0);
    assert_eq!(f.jobs(), before);
}

#[test]
fn changed_ingress_during_hold_insert_rolls_back_every_effect() {
    let f = Fixture::new();
    let before = f.jobs();
    Connection::open(&f.db).unwrap().execute_batch(
        "CREATE TRIGGER alter_stop_ingress AFTER INSERT ON cdr_execution_holds
         BEGIN UPDATE discord_ingress_journal SET owner_user_id=21 WHERE ingress_id='message:101'; END;"
    ).unwrap();
    assert!(accept_nonrunning(&f.db, f.scope(), &f.binding, Some(&f.record), || Ok(())).is_err());
    assert_eq!(f.hold_count(), 0);
    assert_eq!(f.jobs(), before);
    assert_eq!(
        ingress::get(&f.db, "message:101").unwrap().as_ref(),
        Some(&f.record)
    );
}

#[test]
fn failed_or_silently_ignored_hold_insert_never_acknowledges_stop() {
    for effect in ["ABORT", "IGNORE"] {
        let f = Fixture::new();
        let before = f.jobs();
        let raise = if effect == "ABORT" {
            "RAISE(ABORT,'injected')"
        } else {
            "RAISE(IGNORE)"
        };
        Connection::open(&f.db).unwrap().execute_batch(&format!(
            "CREATE TRIGGER fail_hold BEFORE INSERT ON cdr_execution_holds BEGIN SELECT {raise}; END;"
        )).unwrap();
        assert!(
            accept_nonrunning(&f.db, f.scope(), &f.binding, Some(&f.record), || Ok(())).is_err()
        );
        assert_eq!(f.hold_count(), 0);
        assert_eq!(f.jobs(), before);
        assert_eq!(
            ingress::get(&f.db, "message:101").unwrap().as_ref(),
            Some(&f.record)
        );
    }
}

#[test]
fn preexisting_hold_evidence_is_not_replaced() {
    let f = Fixture::new();
    Connection::open(&f.db).unwrap().execute(
        "INSERT INTO cdr_execution_holds VALUES ('original','target','prior uncertainty','original evidence',1)",
        [],
    ).unwrap();
    accept_nonrunning(&f.db, f.scope(), &f.binding, Some(&f.record), || Ok(()))
        .unwrap()
        .unwrap();
    let saved: (String, String, f64) = Connection::open(&f.db).unwrap().query_row(
        "SELECT reason,evidence_json,created_at FROM cdr_execution_holds WHERE job_id='original'",
        [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).unwrap();
    assert_eq!(
        saved,
        ("prior uncertainty".into(), "original evidence".into(), 1.0)
    );
}

#[test]
fn selected_snapshot_change_rolls_back_acceptance() {
    let f = Fixture::new();
    let before = f.jobs();
    let checks = AtomicUsize::new(0);
    let result = accept_nonrunning(&f.db, f.scope(), &f.binding, Some(&f.record), || {
        if checks.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(())
        } else {
            Err(StoreError::Integrity("selected target changed".into()))
        }
    });
    assert!(result.is_err());
    assert_eq!(f.hold_count(), 0);
    assert_eq!(f.jobs(), before);
    assert_eq!(
        ingress::get(&f.db, "message:101").unwrap().as_ref(),
        Some(&f.record)
    );
}

#[test]
fn busy_database_returns_bounded_failure_without_acceptance() {
    let f = Fixture::new();
    let blocker = Connection::open(&f.db).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = Instant::now();
    let result = accept_nonrunning(&f.db, f.scope(), &f.binding, Some(&f.record), || Ok(()));
    let elapsed = started.elapsed();
    blocker.execute_batch("ROLLBACK").unwrap();
    assert!(result.is_err());
    assert!(
        elapsed < Duration::from_secs(3),
        "busy admission took {elapsed:?}"
    );
    assert_eq!(f.hold_count(), 0);
    assert_eq!(
        ingress::get(&f.db, "message:101").unwrap().as_ref(),
        Some(&f.record)
    );
}
