//! Independent native regressions for Pro P2 and local async/DB isolation.
use super::{Arc, CompletionWorker, observe, process_page};
use crate::{
    app_backend::AppServerTurnBackend, queue_runner::QueueCoordinator, soak::native_fixture,
};
use cdr_app_server::ResidentAppServer;
use cdr_store::{observation_gap as store, queue};
use rusqlite::{Connection, params};
use serde_json::json;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::sync::{Semaphore, broadcast, mpsc, watch};

struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
    worker: Arc<CompletionWorker>,
}
impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("review.sqlite");
        let mut config = native_fixture::config("action");
        config.environment.insert(
            "CDR_ACTION_RPC_LOG".into(),
            temp.path().join("rpc.jsonl").to_string_lossy().into_owned(),
        );
        let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
        crate::idle_release::install(&server, &db).unwrap();
        let queue = Arc::new(QueueCoordinator::new(
            db.clone(),
            Arc::new(AppServerTurnBackend::new(Arc::clone(&server))),
        ));
        let worker = Arc::new(CompletionWorker {
            server,
            queue,
            http: Arc::new(twilight_http::Client::new("fixture".into())),
            commentary_enabled: false,
            history_read_timeout: Duration::from_secs(2),
            commentary: tokio::sync::Mutex::new(
                crate::commentary_stream::CommentaryBuffer::default(),
            ),
            terminal_fence: super::super::terminal_fence::TerminalFence::default(),
        });
        Self {
            _temp: temp,
            db,
            worker,
        }
    }
    fn seed(&self) {
        let generation = i64::try_from(self.worker.server.generation()).unwrap();
        queue::enqueue(
            &self.db,
            queue::NewQueueJob {
                job_id: "original",
                target_thread_id: "A",
                channel_id: 42,
                owner_user_id: Some(1),
                discord_message_id: None,
                app_server_generation: generation,
                prompt: "original",
                queued: false,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        queue::begin_attempt(&self.db, "original", &[], generation).unwrap();
        queue::mark_running(&self.db, "original", "T1", generation).unwrap();
    }
    async fn emit(&self) {
        self.worker
            .server
            .request(
                "test/active-turn",
                json!({"threadId":"A","turnId":"T1"}),
                Duration::from_secs(2),
                Some(self.worker.server.generation()),
            )
            .await
            .unwrap();
    }
}

fn assert_discovery_prefix(db: &Connection) {
    let unknown: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM cdr_observation_gaps WHERE first_seq=0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let seen: i64 = db
        .query_row(
            "SELECT seen_seq FROM cdr_observation_streams WHERE active=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        unknown, 0,
        "a delayed exact source snapshot is not an unidentified observation loss"
    );
    assert_eq!(seen, 101);
}

#[tokio::test]
async fn patch06_review_p2_delayed_actual_source_page_does_not_create_unknown_hole() {
    let f = Fixture::new().await;
    f.seed();
    let scope = super::gap::scope(&f.worker).unwrap();
    for _ in 0..96 {
        f.emit().await;
    }
    let g1 = f
        .worker
        .server
        .observation_window(f.worker.server.generation(), 0, None)
        .unwrap();
    super::gap::discover(&f.worker, &scope, &g1).unwrap();
    for original in g1.events.iter().take(2) {
        super::gap::certify_event(
            &f.worker,
            &scope,
            original.sequence,
            original.notification.as_ref().unwrap(),
        )
        .unwrap();
    }
    for _ in 0..4 {
        f.emit().await;
    }
    // Exact ordering barrier: hold the intake's original snapshot while the
    // independent reconciler's newer window commits its discovery first.
    let older = f
        .worker
        .server
        .observation_window(f.worker.server.generation(), 96, None)
        .unwrap();
    assert_eq!(older.source_upper, 100);
    f.emit().await;
    let newer = f
        .worker
        .server
        .observation_window(f.worker.server.generation(), 96, None)
        .unwrap();
    super::gap::discover(&f.worker, &scope, &newer).unwrap();
    assert_eq!(newer.source_upper, 101);
    let (sender, _pending) = mpsc::channel(8);
    let budget = Arc::new(Semaphore::new(super::super::scheduler::lanes::EVENT_BYTES));
    assert!(process_page(&f.worker, &scope, &older, &sender, &budget).await);
    let db = Connection::open(&f.db).unwrap();
    assert_discovery_prefix(&db);
    let (last,proof):(i64,String)=db.query_row(
        "SELECT last_seq,verified_json FROM cdr_observation_gaps WHERE owner_id=?1 AND generation=?2 AND first_seq=1",
        params![scope.owner_id,scope.generation],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(last, 96);
    assert_eq!(
        serde_json::from_str::<Vec<store::Span>>(&proof).unwrap(),
        vec![store::Span { first: 1, last: 2 }]
    );
    let last_g2: i64 = db
        .query_row(
            "SELECT last_seq FROM cdr_observation_gaps WHERE first_seq=97",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(last_g2, 101);
    let (stop, shutdown) = watch::channel(false);
    let task = tokio::spawn(super::super::observation_gap::run(
        Arc::clone(&f.worker),
        shutdown,
    ));
    let verified = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if store::scope_verified(&f.db, &scope, 101).unwrap() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    let _ = stop.send(true);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert!(
        verified.is_ok(),
        "all retained original effects must regain the prefix without an invented unknown"
    );
    assert!(
        f.worker
            .server
            .reconcile_idle_observation_prefix(f.worker.server.generation(), 101)
            .unwrap()
    );
    f.worker.server.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn patch06_review_p3_source_activation_does_not_block_async_progress_on_db_writer() {
    let f = Fixture::new().await;
    let path = f.db.clone();
    let (ready, ready_rx) = tokio::sync::oneshot::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let blocker = std::thread::spawn(move || {
        let db = Connection::open(path).unwrap();
        db.execute_batch("BEGIN IMMEDIATE").unwrap();
        ready.send(()).unwrap();
        let _ = release_rx.recv_timeout(Duration::from_secs(2));
        db.execute_batch("ROLLBACK").unwrap();
    });
    ready_rx.await.unwrap();
    let (wake, receiver) = broadcast::channel(8);
    let (sender, _pending) = mpsc::channel(8);
    let budget = Arc::new(Semaphore::new(super::super::scheduler::lanes::EVENT_BYTES));
    let (stop, shutdown) = watch::channel(false);
    let started = Instant::now();
    let task = tokio::spawn(observe(
        Arc::clone(&f.worker),
        receiver,
        sender,
        budget,
        shutdown,
    ));
    tokio::time::sleep(Duration::from_millis(25)).await;
    let heartbeat = started.elapsed();
    let _ = release.send(());
    blocker.join().unwrap();
    let _ = stop.send(true);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    drop(wake);
    f.worker.server.close().await.unwrap();
    assert!(
        heartbeat < Duration::from_secs(1),
        "source activation blocked the async executor behind an unrelated DB writer for {heartbeat:?}"
    );
}
