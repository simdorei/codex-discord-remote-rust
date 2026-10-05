//! Native source notifications with the production durable journal installed.
use super::{CancelPage, certify_event, discover, reconcile, scope, store};
use crate::{
    app_backend::AppServerTurnBackend, queue_runner::QueueCoordinator, soak::native_fixture,
};
use cdr_app_server::ResidentAppServer;
use cdr_store::queue;
use rusqlite::{Connection, params};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Semaphore, broadcast, mpsc, watch};

use crate::test_support::completion_lane_http as http_fixture;

async fn unknown_store_failure_keeps_optional_hold(failure: &str) {
    let f = Fixture::new(Arc::new(twilight_http::Client::new("fixture".into()))).await;
    f.seed();
    f.emit("test/active-turn").await;
    let generation = f.worker.server.generation();
    let s = scope(&f.worker).unwrap();
    let page = f
        .worker
        .server
        .observation_window(generation, 0, None)
        .unwrap();
    assert_eq!(page.events.len(), 1);
    discover(&f.worker, &s, &page).unwrap();
    for event in &page.events {
        certify_event(
            &f.worker,
            &s,
            event.sequence,
            event.notification.as_ref().unwrap(),
        )
        .unwrap();
    }
    assert!(store::scope_verified(&f.db, &s, 1).unwrap());
    assert!(
        f.worker
            .server
            .reconcile_idle_observation_prefix(generation, 1)
            .unwrap()
    );
    let original = queue::list(&f.db).unwrap();
    let db = Connection::open(&f.db).unwrap();
    let trigger = match failure {
        "IGNORE" => "CREATE TRIGGER reject_unknown BEFORE INSERT ON cdr_observation_gaps
            WHEN NEW.first_seq=0 BEGIN SELECT RAISE(IGNORE); END;",
        "ABORT" => "CREATE TRIGGER reject_unknown BEFORE INSERT ON cdr_observation_gaps
            WHEN NEW.first_seq=0 BEGIN SELECT RAISE(ABORT,'injected unknown persistence failure'); END;",
        _ => panic!("unsupported fixture failure"),
    };
    db.execute_batch(trigger).unwrap();
    f.worker.server.mark_idle_observation_gap();
    let cleared_after_failure = f
        .worker
        .server
        .reconcile_idle_observation_prefix(generation, 1)
        .unwrap();
    db.execute_batch("DROP TRIGGER reject_unknown;").unwrap();
    let cleared_after_store_recovers = f
        .worker
        .server
        .reconcile_idle_observation_prefix(generation, 1)
        .unwrap();
    f.worker
        .server
        .request(
            "turn/start",
            json!({"threadId":"B","input":[]}),
            Duration::from_secs(2),
            Some(generation),
        )
        .await
        .unwrap();
    let after = queue::list(&f.db).unwrap();
    let requests = f
        .rpcs()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    f.worker.server.close().await.unwrap();

    assert!(
        !cleared_after_failure && !cleared_after_store_recovers,
        "{failure}: positive source proof cannot clear an unstored unattributed loss; after_failure={cleared_after_failure}, after_store_recovers={cleared_after_store_recovers}"
    );
    assert_eq!(
        after, original,
        "the original request must not be replayed or rewritten"
    );
    let starts = requests
        .iter()
        .filter(|r| r["method"] == "turn/start")
        .collect::<Vec<_>>();
    assert_eq!(
        starts.len(),
        1,
        "only the separately authorized fixture B starts"
    );
    assert_eq!(starts[0]["params"]["threadId"], "B");
    assert!(!requests.iter().any(|r| r["method"] == "thread/unsubscribe"));
}

#[tokio::test]
async fn patch06_review_r1_ignored_unknown_store_cannot_clear_native_gap() {
    unknown_store_failure_keeps_optional_hold("IGNORE").await;
}

#[tokio::test]
async fn patch06_review_r1_aborted_unknown_store_cannot_clear_native_gap() {
    unknown_store_failure_keeps_optional_hold("ABORT").await;
}
struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
    rpc: PathBuf,
    worker: Arc<super::CompletionWorker>,
}
impl Fixture {
    async fn new(http: Arc<twilight_http::Client>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        let rpc = temp.path().join("rpc.jsonl");
        let mut config = native_fixture::config("action");
        config.environment.insert(
            "CDR_ACTION_RPC_LOG".into(),
            rpc.to_string_lossy().into_owned(),
        );
        let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
        crate::idle_release::install(&server, &db).unwrap();
        let queue = Arc::new(QueueCoordinator::new(
            db.clone(),
            Arc::new(AppServerTurnBackend::new(Arc::clone(&server))),
        ));
        let worker = Arc::new(super::CompletionWorker {
            server,
            queue,
            http,
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
            rpc,
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
                prompt: "never replay",
                queued: false,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        queue::begin_attempt(&self.db, "original", &[], generation).unwrap();
        queue::mark_running(&self.db, "original", "T1", generation).unwrap();
    }
    async fn emit(&self, method: &str) {
        self.worker
            .server
            .request(
                method,
                json!({"threadId":"A","turnId":"T1"}),
                Duration::from_secs(2),
                Some(self.worker.server.generation()),
            )
            .await
            .unwrap();
    }
    fn rpcs(&self) -> String {
        std::fs::read_to_string(&self.rpc).unwrap()
    }
}

#[tokio::test]
async fn patch06_required_journal_abort_recovers_retained_source_without_rpc_or_http_replay() {
    let remote = http_fixture::start([], []).await;
    let client = Arc::new(
        twilight_http::Client::builder()
            .token("fixture".into())
            .proxy(remote.address.clone(), true)
            .ratelimiter(None)
            .build(),
    );
    let f = Fixture::new(client).await;
    f.seed();
    f.emit("test/active-turn").await;
    f.emit("test/finish-turn").await;
    let s = scope(&f.worker).unwrap();
    let page = f
        .worker
        .server
        .observation_window(f.worker.server.generation(), 0, None)
        .unwrap();
    assert_eq!(page.events.len(), 2);
    discover(&f.worker, &s, &page).unwrap();
    let db = Connection::open(&f.db).unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_terminal BEFORE INSERT ON codex_observed_completions
        BEGIN SELECT RAISE(ABORT,'injected required journal failure'); END;",
    )
    .unwrap();
    let mut rejected = 0;
    for e in &page.events {
        if certify_event(&f.worker, &s, e.sequence, e.notification.as_ref().unwrap()).is_err() {
            rejected += 1;
        }
    }
    assert_eq!(rejected, 1);
    f.worker
        .server
        .mark_source_observation_gap(f.worker.server.generation());
    assert!(!store::scope_verified(&f.db, &s, i64::try_from(page.upper).unwrap()).unwrap());
    assert!(
        !f.worker
            .server
            .reconcile_idle_observation_prefix(f.worker.server.generation(), page.upper)
            .unwrap()
    );
    let before = f.rpcs();
    db.execute_batch("DROP TRIGGER reject_terminal;").unwrap();
    for _ in 0..4 {
        reconcile(&f.worker, &s, &AtomicBool::new(false)).unwrap();
    }
    assert!(store::scope_verified(&f.db, &s, i64::try_from(page.upper).unwrap()).unwrap());
    assert!(
        f.worker
            .server
            .reconcile_idle_observation_prefix(f.worker.server.generation(), page.upper)
            .unwrap()
    );
    assert_eq!(
        before,
        f.rpcs(),
        "reconciliation may only replay idempotent local store effects"
    );
    let posts: Vec<_> = remote
        .posts
        .lock()
        .unwrap()
        .iter()
        .map(|post| (post.channel, post.content.clone()))
        .collect();
    assert!(posts.is_empty());
    let original = queue::list(&f.db).unwrap();
    assert_eq!(original.len(), 1);
    assert_eq!(original[0].attempt_count, 1);
    assert_eq!(original[0].turn_id.as_deref(), Some("T1"));
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM codex_observed_completions
        WHERE thread_id='A' AND turn_id='T1' AND resident_owner=?",
            [f.worker.server.instance_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    f.worker.server.close().await.unwrap();
    remote.release();
    remote.close().await;
}

#[tokio::test]
async fn patch06_indexed_intake_forwards_identical_native_occurrences_once_each() {
    let f = Fixture::new(Arc::new(twilight_http::Client::new("fixture".into()))).await;
    f.seed();
    f.emit("test/active-turn").await;
    f.emit("test/active-turn").await;
    let before = f.rpcs();
    let (wake, receiver) = broadcast::channel(2);
    for _ in 0..8 {
        wake.send(cdr_app_server::ResidentNotificationEvent::Gap {
            generation: f.worker.server.generation(),
            skipped: 999,
        })
        .unwrap();
    }
    let (sender, mut pending) = mpsc::channel(8);
    let budget = Arc::new(Semaphore::new(super::super::scheduler::lanes::EVENT_BYTES));
    let (stop, shutdown) = watch::channel(false);
    let worker = Arc::clone(&f.worker);
    let task = tokio::spawn(super::super::source_driver::observe(
        worker, receiver, sender, budget, shutdown,
    ));
    for _ in 0..2 {
        tokio::time::timeout(Duration::from_secs(5), pending.recv())
            .await
            .unwrap()
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(550)).await;
    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert!(
        pending.try_recv().is_err(),
        "wakeup polls cannot manufacture a third occurrence"
    );
    let s = scope(&f.worker).unwrap();
    assert!(store::scope_verified(&f.db, &s, 2).unwrap());
    let seen: i64 = Connection::open(&f.db)
        .unwrap()
        .query_row(
            "SELECT seen_seq FROM cdr_observation_streams WHERE active=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        seen, 2,
        "downstream skipped counts cannot allocate original source sequence"
    );
    assert_eq!(before, f.rpcs());
    f.worker.server.close().await.unwrap();
}

#[tokio::test]
async fn patch06_unknown_scope_blocks_optional_idle_but_not_ordinary_admission() {
    let f = Fixture::new(Arc::new(twilight_http::Client::new("fixture".into()))).await;
    let s = scope(&f.worker).unwrap();
    let db = Connection::open(&f.db).unwrap();
    db.execute(
        "INSERT INTO cdr_idle_release
        (intent_id,owner_id,generation,thread_id,turn_id,job_id,revision,state)
        VALUES('intent',?1,?2,'idle','old-turn','old-job',0,'Candidate')",
        params![s.owner_id, s.generation],
    )
    .unwrap();
    let intent = cdr_store::idle_release::get(&f.db, "idle")
        .unwrap()
        .unwrap();
    cdr_store::idle_release::verify_with_observations(&f.db, &intent, true).unwrap();
    assert!(
        f.worker
            .server
            .reconcile_idle_observation_prefix(f.worker.server.generation(), 0)
            .unwrap()
    );
    f.worker.server.mark_idle_observation_gap();
    assert!(cdr_store::idle_release::verify_with_observations(&f.db, &intent, true).is_err());
    let before = f.rpcs();
    assert!(
        f.worker
            .server
            .release_idle_subscription(crate::idle_release::token(intent).unwrap())
            .await
            .is_err()
    );
    assert_eq!(before, f.rpcs(), "held optional idle sends no unsubscribe");
    f.worker
        .server
        .request(
            "turn/start",
            json!({"threadId":"B","input":[]}),
            Duration::from_secs(2),
            Some(f.worker.server.generation()),
        )
        .await
        .unwrap();
    let requests = f.rpcs();
    assert_eq!(
        requests
            .lines()
            .filter(|l| serde_json::from_str::<serde_json::Value>(l)
                .is_ok_and(|v| v["method"] == "turn/start"))
            .count(),
        1
    );
    f.worker.server.close().await.unwrap();
}

#[tokio::test]
async fn patch06_cancelled_page_does_not_certify_or_rejournal() {
    let f = Fixture::new(Arc::new(twilight_http::Client::new("fixture".into()))).await;
    f.seed();
    f.emit("test/finish-turn").await;
    let s = scope(&f.worker).unwrap();
    store::discover(&f.db, &s, 1).unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    drop(CancelPage(Arc::clone(&cancelled)));
    assert!(cancelled.load(Ordering::Acquire));
    let before = f.rpcs();
    reconcile(&f.worker, &s, &cancelled).unwrap();
    let count: i64 = Connection::open(&f.db)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM codex_observed_completions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    assert!(!store::scope_verified(&f.db, &s, 1).unwrap());
    assert_eq!(before, f.rpcs());
    f.worker.server.close().await.unwrap();
}
