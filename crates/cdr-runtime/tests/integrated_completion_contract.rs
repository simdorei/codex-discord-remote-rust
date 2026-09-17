use cdr_runtime::queue_runner::{
    BackendFailure, BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord,
};
use cdr_store::{delivery, idle_release, observed_completion, queue, reserve_policy};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

struct Backend {
    db: PathBuf,
    mode: u8,
    generation: AtomicU64,
    notes: AtomicUsize,
    starts: AtomicUsize,
}
impl TurnBackend for Backend {
    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }
    fn resident_instance_id(&self) -> Option<&str> {
        Some(if self.generation() == 1 { "one" } else { "two" })
    }
    fn active_turn_id<'a>(&'a self, _target: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }
    fn read_turns<'a>(&'a self, _target: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn resume_thread<'a>(&'a self, _target: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn start_turn<'a>(
        &'a self,
        _target: &'a str,
        _prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async {
            self.starts.fetch_add(1, Ordering::SeqCst);
            Err(BackendFailure::definite("unexpected start"))
        })
    }
    fn note_usage_limit<'a>(&'a self, _target: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async move {
            self.notes.fetch_add(1, Ordering::SeqCst);
            if self.mode == 1 {
                rusqlite::Connection::open(&self.db)
                    .unwrap()
                    .execute("UPDATE codex_turn_queue SET owner_user_id=77", [])
                    .unwrap();
            }
            if self.mode == 2 {
                self.generation.store(2, Ordering::SeqCst);
            }
            Ok(())
        })
    }
}
fn fixture(mode: u8) -> (tempfile::TempDir, Arc<Backend>, QueueCoordinator<Backend>) {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    queue::enqueue(
        &db,
        queue::NewQueueJob {
            job_id: "job",
            target_thread_id: "thread",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: None,
            app_server_generation: 1,
            prompt: "original",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    queue::begin_attempt(&db, "job", &[], 1).unwrap();
    queue::mark_running(&db, "job", "turn", 1).unwrap();
    observed_completion::record(&db, "thread", "turn", 1, "{}").unwrap();
    reserve_policy::ensure(&db, "thread").unwrap();
    let backend = Arc::new(Backend {
        db: db.clone(),
        mode,
        generation: AtomicU64::new(1),
        notes: AtomicUsize::new(0),
        starts: AtomicUsize::new(0),
    });
    let coordinator = QueueCoordinator::new(db, backend.clone());
    (temp, backend, coordinator)
}

#[tokio::test]
async fn owner_changed_during_usage_preparation_cannot_be_finalized() {
    let (_temp, backend, q) = fixture(1);
    let error = q
        .stage_turn_completion_with_usage_limit("thread", "turn", "Failed", true)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("ownership changed"), "{error}");
    assert_eq!(queue::list(&backend.db).unwrap()[0].owner_user_id, Some(77));
    assert!(delivery::list_pending(&backend.db).unwrap().is_empty());
    assert!(observed_completion::contains(&backend.db, "thread", "turn").unwrap());
    assert!(reserve_policy::usage_failure_unresolved(&backend.db, "thread").unwrap());
    assert_eq!(backend.starts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unknown_completion_generation_cannot_create_idle_release_candidate() {
    let (_temp, backend, q) = fixture(0);
    q.stage_turn_completion("thread", "turn", "Final")
        .await
        .unwrap();
    assert_eq!(delivery::list_pending(&backend.db).unwrap().len(), 1);
    assert!(idle_release::get(&backend.db, "thread").unwrap().is_none());
    assert_eq!(backend.starts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn explicit_current_evidence_can_create_release_but_older_evidence_cannot() {
    for generation in [0, 1] {
        let (_temp, backend, q) = fixture(0);
        q.stage_turn_completion_on_generation("thread", "turn", "Final", generation)
            .await
            .unwrap();
        assert_eq!(
            idle_release::get(&backend.db, "thread").unwrap().is_some(),
            generation == 1
        );
        assert_eq!(delivery::list_pending(&backend.db).unwrap().len(), 1);
    }
}

#[tokio::test]
async fn failed_usage_fence_prevents_both_settings_call_and_completion_consumption() {
    let (_temp, backend, q) = fixture(0);
    reserve_policy::ensure(&backend.db, "thread").unwrap();
    rusqlite::Connection::open(&backend.db).unwrap().execute_batch("CREATE TRIGGER refuse_fence BEFORE UPDATE ON codex_reserve_policy BEGIN SELECT RAISE(ABORT,'fence failure'); END;").unwrap();
    assert!(
        q.stage_turn_completion_with_usage_limit("thread", "turn", "Failed", true)
            .await
            .is_err()
    );
    assert_eq!(backend.notes.load(Ordering::SeqCst), 0);
    assert_eq!(queue::list(&backend.db).unwrap().len(), 1);
    assert!(delivery::list_pending(&backend.db).unwrap().is_empty());
}
