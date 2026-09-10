use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cdr_app_server::outcomes::TurnStatus;
use cdr_runtime::queue_runner::{BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord};
use cdr_store::delivery::{complete as complete_delivery, list_pending};
use cdr_store::queue::{
    NewQueueJob, QueueJobState, begin_attempt, enqueue, list, record_start_failure,
};
use rusqlite::Connection;
use tokio::sync::{Barrier, Mutex};

const HOLD_PREFIX: &str = "[cdr-rust:turn-start-candidates-ambiguous:v1] ";
const NOTICE_ID: &str = "turn-start-candidates-ambiguous:held-job";

struct CandidateBackend {
    turns: Mutex<BTreeMap<String, Vec<TurnRecord>>>,
    reads: AtomicUsize,
    resumes: Mutex<Vec<String>>,
    starts: Mutex<Vec<(String, String)>>,
    read_barrier: Option<Arc<Barrier>>,
}

impl CandidateBackend {
    fn new(turns: Vec<TurnRecord>) -> Self {
        Self {
            turns: Mutex::new(BTreeMap::from([("held-thread".into(), turns)])),
            reads: AtomicUsize::new(0),
            resumes: Mutex::default(),
            starts: Mutex::default(),
            read_barrier: None,
        }
    }

    fn concurrent(turns: Vec<TurnRecord>) -> Self {
        Self {
            read_barrier: Some(Arc::new(Barrier::new(2))),
            ..Self::new(turns)
        }
    }

    async fn replace_held_turns(&self, turns: Vec<TurnRecord>) {
        self.turns.lock().await.insert("held-thread".into(), turns);
    }
}

impl TurnBackend for CandidateBackend {
    fn generation(&self) -> u64 {
        9
    }

    fn active_turn_id<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }

    fn read_turns<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async move {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if thread_id == "held-thread"
                && let Some(barrier) = &self.read_barrier
            {
                barrier.wait().await;
            }
            Ok(self
                .turns
                .lock()
                .await
                .get(thread_id)
                .cloned()
                .unwrap_or_default())
        })
    }

    fn resume_thread<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async move {
            self.resumes.lock().await.push(thread_id.into());
            Ok(())
        })
    }

    fn start_turn<'a>(
        &'a self,
        thread_id: &'a str,
        prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.starts
                .lock()
                .await
                .push((thread_id.into(), prompt.into()));
            Ok(format!("started-{thread_id}"))
        })
    }
}

#[tokio::test]
async fn multiple_candidates_hold_one_target_visibly_while_another_target_progresses() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    seed_expired_starting(&db, true);
    enqueue(&db, job("other-job", "other-thread", "run elsewhere", 3.0)).unwrap();
    let backend = Arc::new(CandidateBackend::new(turns(&[
        "baseline",
        "candidate-b",
        "candidate-a",
        "candidate-b",
    ])));
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    queue.recover().await.unwrap();

    let jobs = list(&db).unwrap();
    let held = find(&jobs, "held-job");
    assert_eq!(held.state, QueueJobState::Starting);
    assert_eq!(held.turn_id, None);
    assert!(held.last_error.starts_with(HOLD_PREFIX));
    assert!(held.last_error.contains("candidate_count=2"));
    assert!(held.last_error.find("candidate-a") < held.last_error.find("candidate-b"));
    assert!(held.last_error.contains("original turn/start timeout"));
    assert_eq!(find(&jobs, "following-job").state, QueueJobState::Pending);
    assert_eq!(find(&jobs, "other-job").state, QueueJobState::Running);
    assert_eq!(
        *backend.starts.lock().await,
        vec![("other-thread".into(), "run elsewhere".into())]
    );
    let first_notice = list_pending(&db).unwrap();
    assert_eq!(first_notice.len(), 1);
    assert_eq!(first_notice[0].delivery_id, NOTICE_ID);
    assert_eq!(first_notice[0].job_id, NOTICE_ID);
    assert_ne!(first_notice[0].job_id, held.job_id);
    assert!(first_notice[0].content.contains("held-job"));
    assert!(first_notice[0].content.contains("held-thread"));
    assert!(first_notice[0].content.contains("held"));

    backend
        .replace_held_turns(turns(&["baseline", "candidate-d", "candidate-c"]))
        .await;
    QueueCoordinator::new(db.clone(), Arc::clone(&backend))
        .recover()
        .await
        .unwrap();
    let refreshed = list_pending(&db).unwrap();
    assert_eq!(refreshed.len(), 1);
    assert!(refreshed[0].content.contains("candidate-c"));
    assert!(refreshed[0].content.contains("candidate-d"));
    assert!(!refreshed[0].content.contains("candidate-a"));

    assert!(complete_delivery(&db, NOTICE_ID).unwrap());
    backend.replace_held_turns(turns(&["baseline"])).await;
    QueueCoordinator::new(db.clone(), Arc::clone(&backend))
        .recover()
        .await
        .unwrap();
    backend
        .replace_held_turns(turns(&["baseline", "only-candidate"]))
        .await;
    QueueCoordinator::new(db.clone(), Arc::clone(&backend))
        .recover()
        .await
        .unwrap();

    assert!(list_pending(&db).unwrap().is_empty());
    let jobs = list(&db).unwrap();
    assert_eq!(find(&jobs, "held-job").state, QueueJobState::Starting);
    assert_eq!(find(&jobs, "held-job").turn_id, None);
    assert!(find(&jobs, "held-job").last_error.starts_with(HOLD_PREFIX));
    assert_eq!(find(&jobs, "following-job").state, QueueJobState::Pending);
    assert_eq!(backend.starts.lock().await.len(), 1);
}

#[tokio::test]
async fn two_coordinators_racing_the_same_snapshot_stage_one_hold_and_notice() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    seed_expired_starting(&db, false);
    let backend = Arc::new(CandidateBackend::concurrent(turns(&[
        "baseline",
        "candidate-b",
        "candidate-a",
    ])));
    let first = QueueCoordinator::new(db.clone(), Arc::clone(&backend));
    let second = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let (first_result, second_result) = tokio::join!(first.recover(), second.recover());

    first_result.unwrap();
    second_result.unwrap();
    assert_eq!(backend.reads.load(Ordering::SeqCst), 2);
    assert!(backend.starts.lock().await.is_empty());
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].state, QueueJobState::Starting);
    assert!(jobs[0].last_error.starts_with(HOLD_PREFIX));
    assert_eq!(list_pending(&db).unwrap().len(), 1);
}

fn seed_expired_starting(db: &std::path::Path, following: bool) {
    enqueue(db, job("held-job", "held-thread", "ambiguous work", 1.0)).unwrap();
    begin_attempt(db, "held-job", &["baseline".into()], 9).unwrap();
    record_start_failure(db, "held-job", 9, "original turn/start timeout", true).unwrap();
    Connection::open(db)
        .unwrap()
        .execute(
            "UPDATE codex_turn_queue SET updated_at = 0 WHERE job_id = 'held-job'",
            [],
        )
        .unwrap();
    if following {
        enqueue(
            db,
            job("following-job", "held-thread", "wait behind hold", 2.0),
        )
        .unwrap();
    }
}

fn turns(ids: &[&str]) -> Vec<TurnRecord> {
    ids.iter()
        .map(|turn_id| TurnRecord {
            turn_id: (*turn_id).into(),
            status: TurnStatus::InProgress,
        })
        .collect()
}

fn job<'a>(id: &'a str, target: &'a str, prompt: &'a str, created_at: f64) -> NewQueueJob<'a> {
    NewQueueJob {
        job_id: id,
        target_thread_id: target,
        channel_id: 70,
        owner_user_id: Some(10),
        discord_message_id: None,
        app_server_generation: 9,
        prompt,
        queued: true,
        ack_sent: true,
        created_at,
    }
}

fn find<'a>(
    jobs: &'a [cdr_store::queue::StoredQueueJob],
    id: &str,
) -> &'a cdr_store::queue::StoredQueueJob {
    jobs.iter().find(|job| job.job_id == id).unwrap()
}
