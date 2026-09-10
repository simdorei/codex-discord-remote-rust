use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use cdr_app_server::outcomes::TurnStatus;
use cdr_runtime::queue_runner::{
    BackendFailure, BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord,
};
use cdr_store::delivery::list_pending;
use cdr_store::queue::{QueueJobState, list};
use tokio::sync::Mutex;

#[derive(Default)]
struct FakeBackend {
    active: Mutex<Option<String>>,
    starts: Mutex<Vec<String>>,
    start_failures: Mutex<VecDeque<BackendFailure>>,
    turns: Mutex<BTreeMap<String, TurnStatus>>,
}

impl TurnBackend for FakeBackend {
    fn generation(&self) -> u64 {
        7
    }

    fn active_turn_id<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { Ok(self.active.lock().await.clone()) })
    }

    fn read_turns<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async {
            Ok(self
                .turns
                .lock()
                .await
                .iter()
                .map(|(turn_id, status)| TurnRecord {
                    turn_id: turn_id.clone(),
                    status: *status,
                })
                .collect())
        })
    }

    fn resume_thread<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn start_turn<'a>(
        &'a self,
        _thread_id: &'a str,
        prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.starts.lock().await.push(prompt.into());
            if let Some(error) = self.start_failures.lock().await.pop_front() {
                return Err(error);
            }
            let turn_id = format!("turn-{}", self.starts.lock().await.len());
            *self.active.lock().await = Some(turn_id.clone());
            self.turns
                .lock()
                .await
                .insert(turn_id.clone(), TurnStatus::InProgress);
            Ok(turn_id)
        })
    }
}

#[tokio::test]
async fn failed_start_records_retryable_or_ambiguous_state_instead_of_losing_the_prompt() {
    for (failure, expected_state) in [
        (BackendFailure::definite("rejected"), QueueJobState::Pending),
        (
            BackendFailure::ambiguous("timeout"),
            QueueJobState::Starting,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("mirror.sqlite");
        let backend = Arc::new(FakeBackend::default());
        backend
            .start_failures
            .lock()
            .await
            .push_back(failure.clone());
        let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

        let submission = coordinator
            .submit("thread-a", 10, 20, None, "must survive")
            .await
            .unwrap();

        assert!(submission.queued);
        assert_eq!(submission.turn_id, None);
        assert_eq!(submission.warning, Some(failure.clone()));
        let jobs = list(&db).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].state, expected_state);
        assert_eq!(jobs[0].prompt, "must survive");
        assert_eq!(jobs[0].last_error, failure.message);
    }
}

#[tokio::test]
async fn same_process_ambiguous_start_is_never_resubmitted_by_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("mirror.sqlite");
    let backend = Arc::new(FakeBackend::default());
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));
    coordinator.recover().await.unwrap();
    backend
        .start_failures
        .lock()
        .await
        .push_back(BackendFailure::ambiguous("response lost"));

    let submission = coordinator
        .submit("thread-a", 10, 20, Some(99), "exactly once")
        .await
        .unwrap();
    assert_eq!(*backend.starts.lock().await, vec!["exactly once"]);
    let report = coordinator.recover().await.unwrap();

    assert_eq!(*backend.starts.lock().await, vec!["exactly once"]);
    assert_eq!(report.unresolved, 1);
    assert_eq!(report.requeued, 0);
    assert_eq!(report.started, 0);
    let [job] = list(&db).unwrap().try_into().unwrap();
    assert_eq!(job.job_id, submission.job_id);
    assert_eq!(job.prompt, "exactly once");
    assert_eq!(job.state, QueueJobState::Starting);
    assert_eq!(job.attempt_count, 1);
}

#[tokio::test]
async fn first_prompt_starts_and_second_prompt_stays_durable_until_completion() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("mirror.sqlite");
    let backend = Arc::new(FakeBackend::default());
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let first = coordinator
        .submit("thread-a", 10, 20, Some(100), "first")
        .await
        .unwrap();
    let second = coordinator
        .submit("thread-a", 10, 20, Some(101), "second")
        .await
        .unwrap();

    assert!(!first.queued);
    assert!(second.queued);
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[0].state, QueueJobState::Running);
    assert_eq!(jobs[1].state, QueueJobState::Pending);
    assert_eq!(*backend.starts.lock().await, vec!["first"]);

    *backend.active.lock().await = None;
    let delivery = coordinator
        .stage_turn_completion("thread-a", "turn-1", "final answer")
        .await
        .unwrap()
        .unwrap();

    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].prompt, "second");
    assert_eq!(jobs[0].state, QueueJobState::Running);
    assert_eq!(*backend.starts.lock().await, vec!["first", "second"]);
    assert_eq!(delivery.content, "final answer");
    assert_eq!(list_pending(&db).unwrap(), vec![delivery]);
}

#[tokio::test]
async fn simultaneous_submissions_cannot_start_two_turns_for_one_thread() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("mirror.sqlite");
    let backend = Arc::new(FakeBackend::default());
    let coordinator = Arc::new(QueueCoordinator::new(db.clone(), Arc::clone(&backend)));

    let tasks = (0..8).map(|index| {
        let coordinator = Arc::clone(&coordinator);
        tokio::spawn(async move {
            coordinator
                .submit("thread-a", 10, 20, Some(200 + index), &format!("p-{index}"))
                .await
        })
    });
    for task in tasks {
        task.await.unwrap().unwrap();
    }

    assert_eq!(backend.starts.lock().await.len(), 1);
    assert_eq!(list(&db).unwrap().len(), 8);
}

#[tokio::test]
async fn durable_running_job_blocks_a_second_start_before_gateway_notification_arrives() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("mirror.sqlite");
    let backend = Arc::new(FakeBackend::default());
    let coordinator = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    coordinator
        .submit("thread-a", 10, 20, Some(301), "first")
        .await
        .unwrap();
    *backend.active.lock().await = None;
    let second = coordinator
        .submit("thread-a", 10, 20, Some(302), "second")
        .await
        .unwrap();

    assert!(second.queued);
    assert_eq!(*backend.starts.lock().await, vec!["first"]);
    assert_eq!(
        list(&db)
            .unwrap()
            .iter()
            .filter(|job| job.state == QueueJobState::Running)
            .count(),
        1
    );
}

#[tokio::test]
async fn busy_status_distinguishes_an_active_turn_from_a_durable_queue_only() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("mirror.sqlite");
    let backend = Arc::new(FakeBackend::default());
    let coordinator = QueueCoordinator::new(db, Arc::clone(&backend));

    assert_eq!(
        coordinator.busy_status("thread-a").await.unwrap(),
        cdr_runtime::queue_runner::BusyStatus {
            busy: false,
            allow_steer: false,
        }
    );
    *backend.active.lock().await = Some("turn-live".into());
    assert_eq!(
        coordinator.busy_status("thread-a").await.unwrap(),
        cdr_runtime::queue_runner::BusyStatus {
            busy: true,
            allow_steer: true,
        }
    );
}
