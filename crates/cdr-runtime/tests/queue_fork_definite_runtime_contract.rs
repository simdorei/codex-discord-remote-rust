use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use cdr_app_server::outcomes::TurnStatus;
use cdr_runtime::queue_runner::{
    BackendFailure, BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord,
};
use cdr_store::delivery::list_pending;
use cdr_store::mapping::upsert_thread;
use cdr_store::queue::{
    DEFINITE_FORK_ERROR_PREFIX, NewAppServerForkHandoff, NewQueueJob, QueueJobState,
    begin_app_server_fork_handoff, enqueue, list, record_app_server_fork_failure,
    unresolved_app_server_fork_handoff_for_source,
};
use tokio::sync::Mutex;

#[derive(Default)]
struct DefiniteBackend {
    active: Mutex<BTreeMap<String, String>>,
    forks: Mutex<Vec<String>>,
    fork_results: Mutex<VecDeque<Result<String, BackendFailure>>>,
    starts: Mutex<Vec<(String, String)>>,
    turns: Mutex<BTreeMap<String, TurnStatus>>,
}

impl TurnBackend for DefiniteBackend {
    fn generation(&self) -> u64 {
        9
    }

    fn requires_app_server_fork(&self) -> bool {
        true
    }

    fn active_turn_id<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async move { Ok(self.active.lock().await.get(thread_id).cloned()) })
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

    fn fork_thread<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.forks.lock().await.push(thread_id.into());
            self.fork_results
                .lock()
                .await
                .pop_front()
                .expect("the test supplies every fork result")
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
            let turn_id = format!("turn-{thread_id}");
            self.active
                .lock()
                .await
                .insert(thread_id.into(), turn_id.clone());
            self.turns
                .lock()
                .await
                .insert(turn_id.clone(), TurnStatus::InProgress);
            Ok(turn_id)
        })
    }
}

#[tokio::test]
async fn definite_fork_failure_atomically_releases_fence_and_stages_one_truthful_warning() {
    let (_temp, db) = fixture();
    let backend = Arc::new(DefiniteBackend::default());
    backend.fork_results.lock().await.extend([
        Err(BackendFailure::definite("thread/fork rejected exactly")),
        Ok("managed".into()),
    ]);
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let error = queue
        .ensure_app_server_only_target("source")
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("thread/fork rejected exactly"));
    assert!(
        unresolved_app_server_fork_handoff_for_source(&db, "source")
            .unwrap()
            .is_none()
    );
    let job = list(&db).unwrap().remove(0);
    assert!(job.last_error.starts_with(DEFINITE_FORK_ERROR_PREFIX));
    assert!(job.last_error.contains("thread/fork rejected exactly"));
    let warnings = list_pending(&db).unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].content.contains("definitely failed"));
    assert!(warnings[0].content.contains("thread/fork rejected exactly"));

    assert_eq!(
        queue
            .ensure_app_server_only_target("source")
            .await
            .unwrap()
            .thread_id,
        "managed"
    );
    assert_eq!(list_pending(&db).unwrap(), warnings);
}

#[tokio::test]
async fn recovery_repairs_legacy_definite_gap_then_forks_and_starts_once_without_duplicate_warning()
{
    let (_temp, db) = fixture();
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "legacy-gap",
            ambiguous_job_id: None,
            source_thread_id: "source",
            expected_generation: 9,
            quarantine_reason: "legacy definite crash gap",
        },
    )
    .unwrap();
    record_app_server_fork_failure(&db, "legacy-gap", "legacy definite rejection", false).unwrap();
    let backend = Arc::new(DefiniteBackend::default());
    backend
        .fork_results
        .lock()
        .await
        .push_back(Ok("managed".into()));
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let first = queue.recover().await.unwrap();
    let second = queue.recover().await.unwrap();

    assert_eq!(first.started, 1);
    assert_eq!(second.started, 0);
    assert_eq!(backend.forks.lock().await.as_slice(), &["source"]);
    assert_eq!(
        backend.starts.lock().await.as_slice(),
        &[("managed".into(), "preserve exactly once".into())]
    );
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].target_thread_id, "managed");
    assert_eq!(jobs[0].state, QueueJobState::Running);
    assert!(
        unresolved_app_server_fork_handoff_for_source(&db, "source")
            .unwrap()
            .is_none()
    );
    let warnings = list_pending(&db).unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].content.contains("legacy definite rejection"));
}

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "source", "project", "Original", 70, 71, 1.0).unwrap();
    enqueue(
        &db,
        NewQueueJob {
            job_id: "pending",
            target_thread_id: "source",
            channel_id: 71,
            owner_user_id: Some(10),
            discord_message_id: Some(72),
            app_server_generation: 9,
            prompt: "preserve exactly once",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    (temp, db)
}
