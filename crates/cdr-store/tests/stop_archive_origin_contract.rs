use cdr_store::{
    ingress::stop::{StopScope, accept_nonrunning, revision},
    queue::{self, NewQueueJob},
};
use rusqlite::Connection;
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
        cdr_store::schema::open_initialized(&db).unwrap();
        Self { _temp: temp, db }
    }
    fn valid(&self, method: &str, target: Option<&str>, origin: &Value) -> bool {
        let mut db = Connection::open(&self.db).unwrap();
        let tx = db.transaction().unwrap();
        revision::validate_request_in(&tx, method, target, Some(origin)).is_ok()
    }
    fn stop(&self, target: &str) {
        queue::enqueue(
            &self.db,
            NewQueueJob {
                job_id: target,
                target_thread_id: target,
                channel_id: 99,
                owner_user_id: Some(20),
                discord_message_id: None,
                app_server_generation: 7,
                prompt: "input",
                queued: true,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        accept_nonrunning(
            &self.db,
            StopScope {
                target,
                channel: 99,
                owner: 20,
            },
            &json!({"target":target,"route":"Explicit","command":{"Stop":{"reference":target}}}),
            None,
            || Ok(()),
        )
        .unwrap()
        .unwrap();
    }
}
fn origin() -> Value {
    json!({"target":"root","stopRevision":0,"archiveTargets":["child","root"]})
}

#[test]
fn archive_scope_allows_only_verified_member_resumes_and_root_archive() {
    let f = Fixture::new();
    for (method, target) in [
        ("thread/resume", "root"),
        ("thread/resume", "child"),
        ("thread/archive", "root"),
    ] {
        assert!(f.valid(method, Some(target), &origin()));
    }
    for (method, target) in [
        ("thread/resume", "foreign"),
        ("thread/archive", "child"),
        ("thread/settings/update", "root"),
        ("turn/start", "child"),
        ("mcpServer/tool/call", "root"),
    ] {
        assert!(!f.valid(method, Some(target), &origin()));
    }
    assert!(!f.valid("thread/resume", None, &origin()));
    assert!(
        revision::validate_in(
            &Connection::open(&f.db).unwrap(),
            Some("child"),
            Some(&origin())
        )
        .is_err(),
        "ordinary exact-target parsing is not relaxed"
    );
}

#[test]
fn every_member_is_revalidated_against_the_same_original_revision() {
    for target in ["root", "child"] {
        let f = Fixture::new();
        f.stop(target);
        for (method, member) in [
            ("thread/resume", "root"),
            ("thread/resume", "child"),
            ("thread/archive", "root"),
        ] {
            assert!(
                !f.valid(method, Some(member), &origin()),
                "{target}: {method} {member}"
            );
        }
        let unrelated = json!({"target":"other","stopRevision":0});
        assert!(f.valid("thread/resume", Some("other"), &unrelated));
    }
}

#[test]
fn malformed_or_forged_archive_scope_fails_without_widening_generic_authority() {
    let f = Fixture::new();
    let mut values = vec![
        json!({"target":"root","stopRevision":0,"archiveTargets":[]}),
        json!({"target":"root","stopRevision":0,"archiveTargets":["child"]}),
        json!({"target":"root","stopRevision":0,"archiveTargets":["root","root"]}),
        json!({"target":"root","stopRevision":0,"archiveTargets":["root",null]}),
        json!({"target":"root","stopRevision":0,"archiveTargets":["root"," child"]}),
        json!({"target":"root","stopRevision":1,"archiveTargets":["root"]}),
        json!({"target":"root","stopRevision":-1,"archiveTargets":["root"]}),
        json!({"target":"root","stopRevision":"0","archiveTargets":["root"]}),
        json!({"target":null,"stopRevision":0,"archiveTargets":["root"]}),
        json!({"target":"root","stopRevision":0,"archiveTargets":null}),
    ];
    let mut extra = origin();
    extra["extra"] = json!(true);
    values.push(extra);
    let mut oversized = origin();
    oversized["archiveTargets"] =
        json!((0..102).map(|n| format!("member-{n}")).collect::<Vec<_>>());
    values.push(oversized);
    for value in values {
        assert!(!f.valid("thread/resume", Some("root"), &value), "{value}");
    }
    assert!(
        revision::validate_request_in(
            &Connection::open(&f.db).unwrap(),
            "thread/resume",
            Some("root"),
            Some(&origin())
        )
        .is_err(),
        "requires one database snapshot"
    );
}

#[test]
fn legacy_and_generic_origins_keep_the_original_contract() {
    let f = Fixture::new();
    let ordinary = json!({"target":"root","stopRevision":0});
    assert!(f.valid("thread/settings/update", Some("root"), &ordinary));
    assert!(!f.valid("thread/settings/update", Some("child"), &ordinary));
    f.stop("root");
    let mut db = Connection::open(&f.db).unwrap();
    let tx = db.transaction().unwrap();
    assert!(revision::validate_request_in(&tx, "thread/resume", Some("root"), None).is_err());
    assert!(revision::validate_request_in(&tx, "thread/resume", Some("other"), None).is_ok());
}

#[test]
fn missing_member_revision_history_is_not_a_clean_subtree() {
    let f = Fixture::new();
    f.stop("child");
    Connection::open(&f.db)
        .unwrap()
        .execute(
            "DELETE FROM cdr_stop_revisions WHERE target_thread_id='child'",
            [],
        )
        .unwrap();
    assert!(!f.valid("thread/archive", Some("root"), &origin()));
}
