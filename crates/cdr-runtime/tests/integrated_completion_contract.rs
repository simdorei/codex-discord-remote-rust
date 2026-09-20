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
    generation: AtomicU64,
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
}
fn fixture() -> (tempfile::TempDir, Arc<Backend>, QueueCoordinator<Backend>) {
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
    let backend = Arc::new(Backend {
        db: db.clone(),
        generation: AtomicU64::new(1),
        starts: AtomicUsize::new(0),
    });
    let coordinator = QueueCoordinator::new(db, backend.clone());
    (temp, backend, coordinator)
}

#[tokio::test]
async fn changed_exact_owner_cannot_be_finalized_by_an_older_completion() {
    let (_temp, backend, q) = fixture();
    let original = queue::list(&backend.db).unwrap().remove(0);
    rusqlite::Connection::open(&backend.db)
        .unwrap()
        .execute("UPDATE codex_turn_queue SET owner_user_id=77", [])
        .unwrap();
    let error = q
        .stage_owned_turn_completion(&original, "Failed", true)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("ownership changed"), "{error}");
    assert_eq!(queue::list(&backend.db).unwrap()[0].owner_user_id, Some(77));
    assert!(delivery::list_pending(&backend.db).unwrap().is_empty());
    assert!(observed_completion::contains(&backend.db, "thread", "turn").unwrap());
    assert!(
        reserve_policy::get(&backend.db, "thread")
            .unwrap()
            .is_none()
    );
    assert_eq!(backend.starts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unknown_completion_generation_cannot_create_idle_release_candidate() {
    let (_temp, backend, q) = fixture();
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
        let (_temp, backend, q) = fixture();
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
async fn terminal_usage_failure_stages_final_without_any_policy_dependency_or_replay() {
    let (_temp, backend, q) = fixture();
    rusqlite::Connection::open(&backend.db).unwrap().execute_batch("CREATE TRIGGER refuse_policy_insert BEFORE INSERT ON codex_reserve_policy BEGIN SELECT RAISE(ABORT,'forbidden policy dependency'); END; CREATE TRIGGER refuse_policy_update BEFORE UPDATE ON codex_reserve_policy BEGIN SELECT RAISE(ABORT,'forbidden policy dependency'); END;").unwrap();
    q.stage_turn_completion_with_usage_limit("thread", "turn", "Failed", true)
        .await
        .unwrap()
        .unwrap();
    q.recover().await.unwrap();
    assert_eq!(backend.starts.load(Ordering::SeqCst), 0);
    assert!(
        reserve_policy::get(&backend.db, "thread")
            .unwrap()
            .is_none()
    );
    assert!(queue::list(&backend.db).unwrap().is_empty());
    assert_eq!(delivery::list_pending(&backend.db).unwrap().len(), 1);
}
