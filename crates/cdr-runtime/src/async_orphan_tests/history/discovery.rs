use super::*;
use cdr_store::completion_work::{self, Cursor, Source};

fn add_unprovable_orphans(f: &HistoryFixture, count: usize) {
    let mut db = cdr_store::schema::open_initialized(&f.db).unwrap();
    let tx = db.transaction().unwrap();
    for index in 0..count {
        tx.execute(
            "INSERT INTO cdr_async_execution_obligations
             (question_id,thread_id,origin_job_id,turn_id,channel_id,format_version,revision,
              answer_state,execution_state,admission_state,policy,original_seal,claim_json,
              owner_json,original_error,created_at,updated_at)
             SELECT ?1,?2,?3,turn_id,channel_id,format_version,revision,answer_state,
              execution_state,admission_state,policy,original_seal,claim_json,owner_json,
              original_error,created_at,updated_at
             FROM cdr_async_execution_obligations WHERE question_id=?4",
            rusqlite::params![
                format!("unprovable-{index:03}"),
                format!("a-held-{index:03}"),
                format!("missing-{index:03}"),
                f.id
            ],
        )
        .unwrap();
    }
    tx.commit().unwrap();
}

fn owning_queue(
    f: &HistoryFixture,
) -> Arc<QueueCoordinator<crate::app_backend::AppServerTurnBackend>> {
    Arc::new(QueueCoordinator::new_with_admission_gate(
        f.db.clone(),
        f.backend.clone(),
        crate::restart_readiness::drain::AdmissionGate::new(),
    ))
}

async fn run_until_terminal(
    f: &HistoryFixture,
    coordinator: Arc<QueueCoordinator<crate::app_backend::AppServerTurnBackend>>,
) -> bool {
    // Reuse only the test observer connection; each query sees fresh committed state.
    // Production recovery still performs every schema and ownership check.
    let observation = cdr_store::schema::open_initialized(&f.db).unwrap();
    observation.pragma_update(None, "query_only", true).unwrap();
    let remote = crate::test_support::approval_http::start().await;
    let http = Arc::new(
        twilight_http::Client::builder()
            .token("fixture-token".into())
            .proxy(remote.address.clone(), true)
            .ratelimiter(None)
            .build(),
    );
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(crate::completion_worker::run_completion_worker(
        f.server.subscribe_notifications(),
        f.server.clone(),
        coordinator,
        http,
        false,
        Duration::from_secs(2),
        shutdown,
    ));
    let reached = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state:String=observation.query_row(
                "SELECT execution_state FROM cdr_async_execution_obligations WHERE question_id=?",
                [&f.id],|row|row.get(0),
            ).unwrap();
            if state == "terminal" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok();
    let _ = stop.send(true);
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .expect("completion worker failed to stop within its cancellation bound")
        .unwrap();
    remote.stop.send(()).unwrap();
    assert!(
        remote.task.await.unwrap().is_empty(),
        "recovery must not post a user message"
    );
    reached
}

fn terminal_script() -> Value {
    let mut value = script("completed", Some(user_input(0)));
    value["goal_result"] = json!({"goal":null});
    value
}

fn assert_no_replay(f: &HistoryFixture) {
    assert!(queue::list(&f.db).unwrap().is_empty());
    assert!(
        f.calls().iter().all(|v| matches!(
            v["method"].as_str(),
            Some("initialize" | "thread/read" | "thread/turns/list" | "thread/goal/get")
        )),
        "background discovery emitted a mutation RPC"
    );
    assert_eq!(
        aq::get(&f.db, &f.id).unwrap().error,
        "original send timeout"
    );
}

#[tokio::test]
async fn source_inventory_discovers_queue_absent_orphans_beyond_first_128() {
    let f = HistoryFixture::new(&terminal_script()).await;
    queue::complete(&f.db, "next").unwrap();
    add_unprovable_orphans(&f, 130);
    let mut found = std::collections::BTreeSet::new();
    for source in Source::ALL.into_iter().filter(|source| source.is_state()) {
        let mut cursor = Cursor::default();
        for _ in 0..8 {
            let page = completion_work::page(&f.db, source, &mut cursor, f.server.instance_id(), 1)
                .unwrap();
            assert!(page.entries.len() <= completion_work::PAGE_SIZE);
            for entry in page.entries {
                found.insert(entry.target);
            }
            if cursor.finished {
                break;
            }
        }
        assert!(
            cursor.finished,
            "finite discovery pass never reached its high water"
        );
    }
    f.server.close().await.unwrap();
    assert_eq!(
        found.len(),
        131,
        "queue-free obligations are absent from background inventory"
    );
    assert!(found.contains("thread-b"));
    assert_no_replay(&f);
}

#[tokio::test]
async fn production_worker_settles_orphan_without_queue_or_incoming_event() {
    let f = HistoryFixture::new(&terminal_script()).await;
    queue::complete(&f.db, "next").unwrap();
    let reached = run_until_terminal(&f, owning_queue(&f)).await;
    f.server.close().await.unwrap();
    assert!(
        reached,
        "queue-free orphan was never reconciled by the production worker"
    );
    assert_eq!(f.obligation().2, "settled");
    assert_eq!(aq::get(&f.db, &f.id).unwrap().state, "closed_unknown");
    assert_no_replay(&f);
}

#[tokio::test]
async fn held_prefix_does_not_starve_later_orphan_in_production_worker() {
    let f = HistoryFixture::new(&terminal_script()).await;
    queue::complete(&f.db, "next").unwrap();
    add_unprovable_orphans(&f, 130);
    let reached = run_until_terminal(&f, owning_queue(&f)).await;
    f.server.close().await.unwrap();
    assert!(
        reached,
        "first 128 held obligations starved the later valid orphan"
    );
    let held:i64=cdr_store::schema::open_initialized(&f.db).unwrap().query_row(
        "SELECT COUNT(*) FROM cdr_async_execution_obligations
         WHERE question_id LIKE 'unprovable-%' AND execution_state='unresolved' AND admission_state='held'",
        [],|r|r.get(0)).unwrap();
    assert_eq!(held, 130);
    assert_no_replay(&f);
}

#[tokio::test]
async fn three_busy_target_locks_do_not_occupy_all_native_discovery_slots() {
    let f = HistoryFixture::new(&terminal_script()).await;
    queue::complete(&f.db, "next").unwrap();
    add_unprovable_orphans(&f, 3);
    let coordinator = owning_queue(&f);
    let mut guards = Vec::new();
    for index in 0..3 {
        guards.push(
            coordinator
                .target_lock(&format!("a-held-{index:03}"))
                .unwrap()
                .lock_owned()
                .await,
        );
    }
    let reached = run_until_terminal(&f, coordinator).await;
    drop(guards);
    f.server.close().await.unwrap();
    assert!(
        reached,
        "busy locks consumed all native slots and blocked the independent target"
    );
    assert_no_replay(&f);
}

#[tokio::test]
async fn closed_maintenance_controls_hold_discovery_until_authorized_release() {
    let f = HistoryFixture::new(&terminal_script()).await;
    queue::complete(&f.db, "next").unwrap();
    let gate = crate::restart_readiness::drain::AdmissionGate::new();
    let key = crate::restart_readiness::drain::DrainFenceKey::new("fixture", "1|1", "maintenance")
        .unwrap();
    gate.seal(&key).unwrap();
    gate.close_controls(&key).unwrap();
    let coordinator = Arc::new(QueueCoordinator::new_with_admission_gate(
        f.db.clone(),
        f.backend.clone(),
        gate.clone(),
    ));
    assert!(!run_until_terminal(&f, coordinator.clone()).await);
    assert_eq!(f.obligation().1, "unresolved");
    assert_eq!(
        f.calls()
            .iter()
            .filter(|v| v["method"] != "initialize")
            .count(),
        0
    );
    assert!(gate.is_drained_for(&key));
    assert!(gate.release(&key));
    let reached = run_until_terminal(&f, coordinator).await;
    f.server.close().await.unwrap();
    assert!(reached);
    assert_no_replay(&f);
}

#[tokio::test]
async fn publishing_terminal_remains_held_and_leaves_execution_discovery_inventory() {
    let f = HistoryFixture::new(&terminal_script()).await;
    queue::complete(&f.db, "next").unwrap();
    cdr_store::schema::open_initialized(&f.db).unwrap().execute(
        "UPDATE cdr_async_execution_obligations SET policy='publishing_recovery' WHERE question_id=?",
        [&f.id]).unwrap();
    let reached = run_until_terminal(&f, owning_queue(&f)).await;
    f.server.close().await.unwrap();
    assert!(reached);
    assert_eq!(f.obligation().2, "held");
    assert!(cdr_store::async_resolution::admission_held(&f.db, "thread-b").unwrap());
    let page = completion_work::page(
        &f.db,
        Source::AsyncOrphan,
        &mut Cursor::default(),
        f.server.instance_id(),
        1,
    )
    .unwrap();
    assert!(page.entries.is_empty());
    assert!(cdr_store::async_resolution::guard_mutation(&f.db, "thread-b").is_err());
    assert_no_replay(&f);
}

#[tokio::test]
async fn lost_terminal_read_keeps_hold_and_other_reads_work_after_worker_shutdown() {
    let mut value = terminal_script();
    value["terminal_timeout"] = json!(true);
    let f = HistoryFixture::new(&value).await;
    queue::complete(&f.db, "next").unwrap();
    assert!(!run_until_terminal(&f, owning_queue(&f)).await);
    assert_eq!(f.obligation().1, "unresolved");
    assert_eq!(f.obligation().2, "held");
    let observed = f
        .server
        .execute(
            AppRequest {
                method: "thread/read",
                params: json!({"threadId":"other-thread","includeTurns":false}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(observed["thread"]["id"], "other-thread");
    f.server.close().await.unwrap();
    assert_no_replay(&f);
}

#[tokio::test]
async fn shutdown_during_terminal_read_releases_lease_and_cold_worker_reconciles_fresh() {
    let mut value = terminal_script();
    value["terminal_gate"] = json!(true);
    let f = HistoryFixture::new(&value).await;
    queue::complete(&f.db, "next").unwrap();
    let coordinator = owning_queue(&f);
    let remote = crate::test_support::approval_http::start().await;
    let http = Arc::new(
        twilight_http::Client::builder()
            .token("fixture-token".into())
            .proxy(remote.address.clone(), true)
            .ratelimiter(None)
            .build(),
    );
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(crate::completion_worker::run_completion_worker(
        f.server.subscribe_notifications(),
        f.server.clone(),
        coordinator.clone(),
        http,
        false,
        Duration::from_secs(2),
        shutdown,
    ));
    wait_file(&f.temp.path().join("rpc.jsonl.terminal-entered")).await;
    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(f.obligation().1, "unresolved");
    let lock = coordinator.target_lock("thread-b").unwrap();
    assert!(
        lock.try_lock().is_ok(),
        "cancelled worker retained the original target mutex"
    );
    remote.stop.send(()).unwrap();
    assert!(remote.task.await.unwrap().is_empty());
    std::fs::write(
        f.temp.path().join("rpc.jsonl.terminal-release"),
        b"release cancelled read",
    )
    .unwrap();
    let reached = run_until_terminal(&f, owning_queue(&f)).await;
    f.server.close().await.unwrap();
    assert!(reached);
    assert_eq!(f.obligation().4, 1);
    assert_no_replay(&f);
}

#[tokio::test]
async fn discovery_high_water_and_metadata_bound_survive_new_later_and_oversized_targets() {
    let f = HistoryFixture::new(&terminal_script()).await;
    queue::complete(&f.db, "next").unwrap();
    add_unprovable_orphans(&f, 130);
    let mut cursor = Cursor::default();
    let first = completion_work::page(
        &f.db,
        Source::AsyncOrphan,
        &mut cursor,
        f.server.instance_id(),
        1,
    )
    .unwrap();
    assert_eq!(first.entries.len(), completion_work::PAGE_SIZE);
    let db = cdr_store::schema::open_initialized(&f.db).unwrap();
    for (id, target) in [
        ("late", "zz-future".to_owned()),
        ("oversized", "x".repeat(5000)),
    ] {
        db.execute(
            "INSERT INTO cdr_async_execution_obligations
             (question_id,thread_id,origin_job_id,turn_id,channel_id,format_version,revision,
              answer_state,execution_state,admission_state,policy,original_seal,claim_json,
              owner_json,original_error,created_at,updated_at)
             SELECT ?1,?2,?1,turn_id,channel_id,format_version,revision,answer_state,
              execution_state,admission_state,policy,original_seal,claim_json,owner_json,
              original_error,created_at,updated_at
             FROM cdr_async_execution_obligations WHERE question_id=?3",
            rusqlite::params![id, target, f.id],
        )
        .unwrap();
    }
    let mut seen = first
        .entries
        .into_iter()
        .map(|e| e.target)
        .collect::<Vec<_>>();
    let mut oversized = false;
    for _ in 0..8 {
        let page = completion_work::page(
            &f.db,
            Source::AsyncOrphan,
            &mut cursor,
            f.server.instance_id(),
            1,
        )
        .unwrap();
        oversized |= page.oversized_identity;
        assert!(page.entries.len() <= completion_work::PAGE_SIZE);
        seen.extend(page.entries.into_iter().map(|e| e.target));
        if cursor.finished {
            break;
        }
    }
    assert!(cursor.finished);
    assert!(oversized);
    assert_eq!(seen.len(), 131);
    assert!(!seen.iter().any(|t| t == "zz-future" || t.len() > 4096));
    let mut fresh = Cursor::default();
    let mut found = false;
    for _ in 0..8 {
        let page = completion_work::page(
            &f.db,
            Source::AsyncOrphan,
            &mut fresh,
            f.server.instance_id(),
            1,
        )
        .unwrap();
        found |= page.entries.iter().any(|e| e.target == "zz-future");
        if fresh.finished {
            break;
        }
    }
    f.server.close().await.unwrap();
    assert!(
        found,
        "new later target was never considered on a fresh finite pass"
    );
    assert_no_replay(&f);
}

#[tokio::test]
async fn ready_capacity_of_busy_locks_does_not_starve_independent_orphan() {
    let f = HistoryFixture::new(&terminal_script()).await;
    queue::complete(&f.db, "next").unwrap();
    add_unprovable_orphans(&f, 128);
    let original_error = f.obligation().3;
    let coordinator = owning_queue(&f);
    let mut guards = Vec::new();
    for index in 0..128 {
        guards.push(
            coordinator
                .target_lock(&format!("a-held-{index:03}"))
                .unwrap()
                .lock_owned()
                .await,
        );
    }
    let reached = run_until_terminal(&f, coordinator).await;
    let intact: i64 = cdr_store::schema::open_initialized(&f.db)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM cdr_async_execution_obligations
         WHERE question_id LIKE 'unprovable-%' AND execution_state='unresolved'
           AND admission_state='held' AND original_error=?",
            [&original_error],
            |r| r.get(0),
        )
        .unwrap();
    drop(guards);
    f.server.close().await.unwrap();
    assert!(
        reached,
        "128 locked durable hints permanently excluded the independent orphan"
    );
    assert_eq!(intact, 128);
    assert_no_replay(&f);
}

#[tokio::test]
async fn preflight_never_authorizes_a_changed_original_obligation() {
    let f = HistoryFixture::new(&terminal_script()).await;
    queue::complete(&f.db, "next").unwrap();
    let negative = completion_work::read_round(&f.db, f.server.instance_id(), 1, |reader| {
        let page = reader.page(Source::AsyncOrphan, &Cursor::default())?;
        reader.unprovable_orphans(&page.page.entries)
    })
    .unwrap()
    .unwrap();
    assert!(negative.is_empty());
    cdr_store::schema::open_initialized(&f.db)
        .unwrap()
        .execute(
            "UPDATE cdr_async_execution_obligations SET revision=-1 WHERE question_id=?",
            [&f.id],
        )
        .unwrap();
    let coordinator = owning_queue(&f);
    let lease = coordinator.try_target_lease("thread-b").unwrap().unwrap();
    let before = f.calls().len();
    assert!(lease.reconcile_orphan_history().await.is_err());
    assert_eq!(
        f.calls().len(),
        before,
        "old preflight must not authorize even a history RPC"
    );
    drop(lease);
    f.server.close().await.unwrap();
    assert_no_replay(&f);
}
