//! Real metadata plus draft rollback; no native or Discord authority is granted.
use super::*;
use std::sync::Arc;

use cdr_app_server::{Notification, ResidentNotificationEvent};
use tokio::sync::Semaphore;

fn seeded(count: usize) -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.sqlite");
    drop(cdr_store::schema::open_initialized(&path).unwrap());
    for index in 0..count {
        let id = format!("read-round-{index:03}");
        cdr_store::queue::enqueue(
            &path,
            cdr_store::queue::NewQueueJob {
                job_id: &id,
                target_thread_id: &id,
                channel_id: 40,
                owner_user_id: Some(1),
                discord_message_id: None,
                app_server_generation: 1,
                prompt: "fixture",
                queued: false,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
    }
    (directory, path)
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

fn state(value: &Scan) -> (Cursor, Instant, bool, VecDeque<completion_work::Entry>) {
    (
        value.cursor.clone(),
        value.next,
        value.wake,
        value.pending.clone(),
    )
}

fn entries(path: &std::path::Path) -> Vec<completion_work::Entry> {
    completion_work::page(path, Source::Queue, &mut Cursor::default(), "runtime", 1)
        .unwrap()
        .entries
}

fn live(target: &str, budget: &Arc<Semaphore>) -> Envelope {
    Envelope::charge(
        ResidentNotificationEvent::Notification {
            generation: 1,
            notification: Notification {
                method: "turn/started".into(),
                params: serde_json::json!({"threadId":target,"turn":{"id":"turn"}}),
            },
        },
        budget,
    )
    .unwrap()
}

#[test]
fn no_due_page_never_opens_a_database() {
    let directory = tempfile::tempdir().unwrap();
    let absent = directory.path().join("absent.sqlite");
    let mut scans = Source::ALL.map(scan);
    for item in &mut scans {
        item.cursor.finished = true;
        item.next = Instant::now() + Duration::from_mins(1);
    }
    let before = scans.iter().map(state).collect::<Vec<_>>();
    let mut ready = Ready::default();
    let mut rotation = 0;
    let reports = read_metadata_round(
        &mut scans,
        &mut rotation,
        &mut ready,
        &HashSet::new(),
        (&absent, "runtime", 1),
    )
    .unwrap();
    assert!(reports.iter().all(|(_, report)| matches!(report, Ok(None))));
    assert_eq!(scans.iter().map(state).collect::<Vec<_>>(), before);
    assert_eq!(rotation, 1);
    assert!(!absent.exists());
}

#[test]
fn common_schema_failure_restores_pending_and_existing_live_work() {
    let (_directory, path) = seeded(2);
    let metadata = entries(&path);
    let mut scans = [scan(Source::Queue), scan(Source::Observed)];
    scans[0].pending.push_back(metadata[0].clone());
    scans[0].cursor.finished = true;
    scans[0].next = Instant::now() + Duration::from_mins(1);
    let before = scans.iter().map(state).collect::<Vec<_>>();
    let budget = Arc::new(Semaphore::new(lanes::EVENT_BYTES));
    let mut ready = Ready::default();
    assert!(ready.live(live("live-original", &budget)));
    let permits = budget.available_permits();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("DROP INDEX codex_mutation_prepared_target")
        .unwrap();
    let mut rotation = 0;
    assert!(
        read_metadata_round(
            &mut scans,
            &mut rotation,
            &mut ready,
            &HashSet::new(),
            (&path, "runtime", 1),
        )
        .is_err()
    );
    assert_eq!(scans.iter().map(state).collect::<Vec<_>>(), before);
    assert_eq!(rotation, 0);
    assert_eq!(ready.state.len(), 1);
    assert_eq!(ready.state[0].target(), "live-original");
    assert_eq!(budget.available_permits(), permits);
}

#[test]
fn discarded_ready_draft_removes_only_speculative_tails() {
    let (_directory, path) = seeded(2);
    let metadata = entries(&path);
    let budget = Arc::new(Semaphore::new(lanes::EVENT_BYTES));
    let mut ready = Ready::default();
    assert!(ready.live(live("live-original", &budget)));
    let mut original_http = metadata[0].clone();
    original_http.source = Source::Final;
    original_http.channel = 40;
    ready.http.push_back(original_http.clone());
    let permits = budget.available_permits();
    {
        let draft = ReadyDraft::new(&mut ready);
        draft.ready.durable(metadata[1].clone(), &HashSet::new());
        let mut new_http = metadata[1].clone();
        new_http.source = Source::Final;
        draft.ready.durable(new_http, &HashSet::new());
        assert_eq!(draft.ready.state.len(), 2);
        assert_eq!(draft.ready.http.len(), 2);
    }
    assert_eq!(ready.state.len(), 1);
    assert_eq!(ready.state[0].target(), "live-original");
    assert_eq!(ready.http, VecDeque::from([original_http]));
    assert_eq!(budget.available_permits(), permits);
    drop(ready);
    assert_eq!(budget.available_permits(), lanes::EVENT_BYTES);
}

#[test]
fn failed_source_preserves_its_pending_but_other_sources_progress() {
    let (_directory, path) = seeded(2);
    let mut retained = entries(&path)[0].clone();
    retained.target = "retained-pending".into();
    let mut scans = [
        scan(Source::Queue),
        scan(Source::Queue),
        scan(Source::Observed),
    ];
    scans[1].pending.push_back(retained);
    scans[1].wake = true;
    let before = state(&scans[1]);
    let mut ready = Ready::default();
    let mut rotation = 0;
    let reports = read_metadata_round(
        &mut scans,
        &mut rotation,
        &mut ready,
        &HashSet::new(),
        (&path, "runtime", 1),
    )
    .unwrap();
    assert!(reports[0].1.is_ok());
    assert!(
        reports[1].1.is_err(),
        "duplicate source is an actual per-source rejection"
    );
    assert!(reports[2].1.is_ok());
    assert_eq!(state(&scans[1]), before);
    assert!(scans[0].cursor.finished && scans[2].cursor.finished);
    assert_eq!(rotation, 1);
    assert_eq!(ready.state.len(), 2);
    assert!(
        ready
            .state
            .iter()
            .all(|work| work.target() != "retained-pending")
    );
}

#[test]
fn full_ready_retains_one_new_page_and_unoffered_page_blocks_another_read() {
    let (_directory, path) = seeded(34);
    let budget = Arc::new(Semaphore::new(lanes::EVENT_BYTES));
    let mut ready = Ready::default();
    for index in 0..lanes::READY_CAP {
        assert!(ready.live(live(&format!("live-{index}"), &budget)));
    }
    let mut scans = [scan(Source::Queue)];
    let mut rotation = 0;
    read_metadata_round(
        &mut scans,
        &mut rotation,
        &mut ready,
        &HashSet::new(),
        (&path, "runtime", 1),
    )
    .unwrap();
    assert_eq!(ready.state.len(), lanes::READY_CAP);
    assert_eq!(scans[0].pending.len(), completion_work::PAGE_SIZE);
    assert!(!scans[0].cursor.finished);
    let before = state(&scans[0]);
    let absent = path.with_file_name("absent.sqlite");
    read_metadata_round(
        &mut scans,
        &mut rotation,
        &mut ready,
        &HashSet::new(),
        (&absent, "runtime", 1),
    )
    .unwrap();
    assert_eq!(state(&scans[0]), before);
    assert!(!absent.exists());
    ready.state.clear();
    read_metadata_round(
        &mut scans,
        &mut rotation,
        &mut ready,
        &HashSet::new(),
        (&path, "runtime", 1),
    )
    .unwrap();
    assert_eq!(ready.state.len(), 34);
    assert!(scans[0].pending.is_empty() && scans[0].cursor.finished);
}

#[test]
fn rotated_sources_keep_fifo_and_active_targets_are_only_hints() {
    let (_directory, path) = seeded(2);
    let budget = Arc::new(Semaphore::new(lanes::EVENT_BYTES));
    let mut ready = Ready::default();
    assert!(ready.live(live("same-live-target", &budget)));
    assert!(ready.live(live("same-live-target", &budget)));
    let mut scans = [scan(Source::Queue), scan(Source::Observed)];
    let active = HashSet::from(["read-round-000".into()]);
    let mut rotation = 1;
    let reports = read_metadata_round(
        &mut scans,
        &mut rotation,
        &mut ready,
        &active,
        (&path, "runtime", 1),
    )
    .unwrap();
    assert_eq!(reports[0].0, Source::Observed);
    assert_eq!(reports[1].0, Source::Queue);
    assert_eq!(rotation, 0);
    assert_eq!(ready.state[0].target(), "same-live-target");
    assert_eq!(ready.state[1].target(), "same-live-target");
    assert_eq!(ready.state[2].target(), "read-round-001");
    assert_eq!(
        entries(&path).len(),
        2,
        "discovery must not change durable rows"
    );
}
