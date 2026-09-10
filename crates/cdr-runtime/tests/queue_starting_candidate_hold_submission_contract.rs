use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cdr_runtime::queue_runner::{
    BackendFailureKind, BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord,
};
use cdr_store::queue::{
    NewQueueJob, begin_attempt, enqueue, hold_starting_for_ambiguous_candidates_if_claimed, list,
};

struct NoCallBackend {
    calls: AtomicUsize,
}

impl TurnBackend for NoCallBackend {
    fn generation(&self) -> u64 {
        9
    }

    fn active_turn_id<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(None) })
    }

    fn read_turns<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(Vec::new()) })
    }

    fn resume_thread<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, ()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }

    fn start_turn<'a>(
        &'a self,
        _thread_id: &'a str,
        _prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok("must-not-start".into()) })
    }
}

#[tokio::test]
async fn same_occurrence_submission_reports_manual_hold_without_replay_or_waiting() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("queue.sqlite");
    enqueue(
        &db,
        NewQueueJob {
            job_id: "held-job",
            target_thread_id: "held-thread",
            channel_id: 70,
            owner_user_id: Some(10),
            discord_message_id: Some(700),
            app_server_generation: 9,
            prompt: "original",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    let starting = begin_attempt(&db, "held-job", &["baseline".into()], 9).unwrap();
    hold_starting_for_ambiguous_candidates_if_claimed(
        &db,
        &starting,
        &["candidate-b".into(), "candidate-a".into()],
    )
    .unwrap()
    .unwrap();
    let before = list(&db).unwrap();
    let backend = Arc::new(NoCallBackend {
        calls: AtomicUsize::new(0),
    });
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let replay = queue
        .submit_identified(
            "new-job-id",
            "held-thread",
            70,
            10,
            Some(700),
            "duplicate delivery",
        )
        .await
        .unwrap();

    assert!(!replay.queued);
    assert_eq!(replay.turn_id, None);
    let warning = replay.warning.expect("held replay must be explicit");
    assert_eq!(warning.kind, BackendFailureKind::StartingCandidatesHeld);
    assert!(warning.message.contains("turn-start-candidates-ambiguous"));
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    assert_eq!(list(&db).unwrap(), before);
}
