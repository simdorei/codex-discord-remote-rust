use super::*;
use cdr_app_server::{Notification, ResidentNotificationEvent};
use cdr_store as store;
use std::sync::Arc;
use tokio::sync::Semaphore;

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::path::PathBuf;

struct Fixture {
    _directory: tempfile::TempDir,
    path: PathBuf,
    db: Connection,
}

fn evidence(id: &str, target: &str, valid: bool) -> (Value, Value) {
    let job = format!("lost-{id}");
    let body = json!({"index":0,"source_text":"","title":"pick","options":["one"]});
    let claim = json!({"id":if valid {id.to_owned()} else {format!("different-{id}")},
        "runtime_id":"runtime","generation":1,"thread_id":target,"turn_id":"turn",
        "item_id":"item","origin_job_id":job,"channel_id":20,"owner_user_id":1,
        "body":body.to_string(),"message_id":"message","chosen":0,"dispatch_mode":"steer"});
    let seal = json!({"identity":{"question":["runtime",target,"turn","item",job],
        "generation":1,"channel":20,"actor":1,"message":"message","chosen":0,
        "body":body,"reply_job_id":null,"job":{"job_id":job,"target_thread_id":target,
        "channel_id":20,"owner_user_id":1,"turn_id":"turn"}}});
    (claim, seal)
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preflight.sqlite");
        let db = store::schema::open_initialized(&path).unwrap();
        Self {
            _directory: directory,
            path,
            db,
        }
    }

    fn seed(&self, id: &str, target: &str, valid: bool) {
        let (claim, seal) = evidence(id, target, valid);
        self.db
            .execute(
                "INSERT INTO cdr_async_execution_obligations
            (question_id,thread_id,origin_job_id,turn_id,channel_id,format_version,revision,
             answer_state,execution_state,admission_state,policy,original_seal,claim_json,
             owner_json,original_error,created_at,updated_at)
            VALUES(?1,?2,?3,'turn',20,1,0,'unresolved','unresolved','held','ordinary',
                ?4,?5,NULL,'original uncertainty',1,1)",
                params![
                    id,
                    target,
                    format!("lost-{id}"),
                    seal.to_string(),
                    claim.to_string()
                ],
            )
            .unwrap();
    }

    // Simulate an out-of-band repair in a disposable fixture only. Restore the
    // exact immutable trigger in the same transaction; no production repair API.
    fn rewrite(&mut self, sql: &str, values: &[&dyn rusqlite::ToSql]) {
        let trigger: String = self
            .db
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name='cdr_async_obligation_claim_immutable'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let tx = self.db.transaction().unwrap();
        tx.execute_batch("DROP TRIGGER cdr_async_obligation_claim_immutable")
            .unwrap();
        tx.execute(sql, values).unwrap();
        tx.execute_batch(&trigger).unwrap();
        tx.commit().unwrap();
    }

    fn repair(&mut self, id: &str, target: &str) {
        let (claim, seal) = evidence(id, target, true);
        self.rewrite("UPDATE cdr_async_execution_obligations SET claim_json=?1,original_seal=?2 WHERE question_id=?3",
            &[&claim.to_string(), &seal.to_string(), &id]);
    }
}

fn scan(source: Source) -> Scan {
    Scan {
        source,
        cursor: Cursor::default(),
        next: Instant::now(),
        wake: false,
        pending: VecDeque::new(),
    }
}

fn live(target: &str, budget: &Arc<Semaphore>) -> Envelope {
    Envelope::charge(
        ResidentNotificationEvent::Notification {
            generation: 1,
            notification: Notification {
                method: "turn/started".into(),
                params: json!({"threadId":target,"turn":{"id":"turn"}}),
            },
        },
        budget,
    )
    .unwrap()
}

fn collect(reports: Vec<DiscoveryResult>) -> DeferredOrphans {
    let mut deferred = DeferredOrphans::default();
    for (_, result) in reports {
        if let Some(page) = result.unwrap() {
            deferred.entries.extend(page.deferred.entries);
        }
    }
    deferred
}

#[test]
fn discovery_keeps_all_hints_and_dispatch_skips_only_exact_new_orphans() {
    let fixture = Fixture::new();
    for index in 0..32 {
        fixture.seed(&format!("q-{index:03}"), &format!("t-{index:03}"), false);
    }
    let mut scans = [scan(Source::AsyncOrphan)];
    let mut ready = Ready::default();
    let budget = Arc::new(Semaphore::new(lanes::EVENT_BYTES));
    assert!(ready.live(live("existing-live", &budget)));
    let permits = budget.available_permits();
    let mut rotation = 0;
    let deferred = collect(
        read_metadata_round(
            &mut scans,
            &mut rotation,
            &mut ready,
            &HashSet::new(),
            (&fixture.path, "runtime", 1),
        )
        .unwrap(),
    );
    assert_eq!(
        ready.state.len(),
        33,
        "all 32 target hints coexist with existing live work"
    );
    assert_eq!(
        ready
            .state
            .iter()
            .filter(|work| matches!(work,
        StateWork::Durable(entry) if entry.source == Source::AsyncOrphan))
            .count(),
        32
    );
    assert_eq!(deferred.entries.len(), 32);
    let mut other = deferred.entries[0].1.clone();
    other.source = Source::Queue;
    ready.state.push_back(StateWork::Durable(other));
    deferred.discard(&mut ready);
    assert_eq!(ready.state.len(), 2);
    assert!(matches!(ready.state[0], StateWork::Live(_)));
    assert!(matches!(&ready.state[1], StateWork::Durable(entry) if entry.source == Source::Queue));
    assert_eq!(
        budget.available_permits(),
        permits,
        "live work and its permit must survive"
    );
}

#[test]
fn full_ready_pending_never_carries_a_stale_negative_after_repair() {
    let mut fixture = Fixture::new();
    fixture.seed("q", "target", false);
    let budget = Arc::new(Semaphore::new(lanes::EVENT_BYTES));
    let mut ready = Ready::default();
    for index in 0..lanes::READY_CAP {
        assert!(ready.live(live(&format!("live-{index}"), &budget)));
    }
    let mut scans = [scan(Source::AsyncOrphan)];
    let mut rotation = 0;
    let old = collect(
        read_metadata_round(
            &mut scans,
            &mut rotation,
            &mut ready,
            &HashSet::new(),
            (&fixture.path, "runtime", 1),
        )
        .unwrap(),
    );
    assert_eq!(scans[0].pending.len(), 1);
    assert!(
        old.entries.is_empty(),
        "pending may retain Entry only, not a negative"
    );
    old.discard(&mut ready);
    fixture.repair("q", "target");
    ready.state.clear();
    let absent = fixture.path.with_file_name("must-not-open.sqlite");
    let current = collect(
        read_metadata_round(
            &mut scans,
            &mut rotation,
            &mut ready,
            &HashSet::new(),
            (&absent, "runtime", 1),
        )
        .unwrap(),
    );
    assert!(current.entries.is_empty());
    current.discard(&mut ready);
    assert_eq!(ready.state.len(), 1);
    assert_eq!(ready.state[0].target(), "target");
    assert!(
        store::async_resolution::capture_history_snapshot(&fixture.path, "target")
            .unwrap()
            .is_some()
    );
}

#[test]
fn repaired_hint_returns_on_next_pass_without_rewinding_the_old_pass() {
    let mut fixture = Fixture::new();
    fixture.seed("q", "target", false);
    let mut scans = [scan(Source::AsyncOrphan)];
    let mut ready = Ready::default();
    let mut rotation = 0;
    collect(
        read_metadata_round(
            &mut scans,
            &mut rotation,
            &mut ready,
            &HashSet::new(),
            (&fixture.path, "runtime", 1),
        )
        .unwrap(),
    )
    .discard(&mut ready);
    assert!(ready.state.is_empty() && scans[0].cursor.finished);
    fixture.repair("q", "target");
    scans[0].wake = true;
    let next = collect(
        read_metadata_round(
            &mut scans,
            &mut rotation,
            &mut ready,
            &HashSet::new(),
            (&fixture.path, "runtime", 1),
        )
        .unwrap(),
    );
    assert!(next.entries.is_empty());
    next.discard(&mut ready);
    assert_eq!(ready.state.len(), 1);
    assert_eq!(ready.state[0].target(), "target");
}

#[test]
fn row_decode_failure_rolls_back_orphan_source_and_preserves_live_work() {
    let fixture = Fixture::new();
    fixture.seed("q", "target", false);
    fixture
        .db
        .execute(
            "UPDATE cdr_async_execution_obligations SET original_error=x'ff'",
            [],
        )
        .unwrap();
    let budget = Arc::new(Semaphore::new(lanes::EVENT_BYTES));
    let mut ready = Ready::default();
    assert!(ready.live(live("other-live", &budget)));
    let mut scans = [scan(Source::AsyncOrphan), scan(Source::Observed)];
    let mut rotation = 0;
    let reports = read_metadata_round(
        &mut scans,
        &mut rotation,
        &mut ready,
        &HashSet::new(),
        (&fixture.path, "runtime", 1),
    )
    .unwrap();
    assert!(reports[0].1.is_err());
    assert!(reports[1].1.is_ok());
    assert_eq!(scans[0].cursor, Cursor::default());
    assert!(scans[0].pending.is_empty());
    assert!(scans[1].cursor.finished);
    assert_eq!(ready.state.len(), 1);
    assert_eq!(ready.state[0].target(), "other-live");
}

#[test]
fn sidecar_does_not_remove_replaced_live_work_at_the_same_position() {
    let fixture = Fixture::new();
    fixture.seed("q", "target", false);
    let mut scans = [scan(Source::AsyncOrphan)];
    let mut ready = Ready::default();
    let mut rotation = 0;
    let deferred = collect(
        read_metadata_round(
            &mut scans,
            &mut rotation,
            &mut ready,
            &HashSet::new(),
            (&fixture.path, "runtime", 1),
        )
        .unwrap(),
    );
    let budget = Arc::new(Semaphore::new(lanes::EVENT_BYTES));
    ready.state[0] = StateWork::Live(live("target", &budget));
    deferred.discard(&mut ready);
    assert_eq!(ready.state.len(), 1);
    assert!(matches!(ready.state[0], StateWork::Live(_)));
}
