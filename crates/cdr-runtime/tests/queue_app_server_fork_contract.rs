use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use cdr_runtime::queue_runner::{
    BackendFailure, BackendFailureKind, BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord,
};
use cdr_store::mapping::{thread_channels, upsert_thread};
use cdr_store::queue::{
    NewAppServerForkHandoff, NewQueueJob, QueueJobState, UNRESOLVED_FORK_ERROR_PREFIX,
    begin_app_server_fork_handoff, begin_attempt, complete_app_server_fork_handoff, enqueue, list,
    record_app_server_fork_failure, record_start_failure, stage_app_server_fork_target,
    unresolved_app_server_fork_handoff_for_source,
};
use tokio::sync::Mutex;

#[derive(Default)]
struct FakeBackend {
    active: Mutex<BTreeMap<String, String>>,
    forks: Mutex<Vec<String>>,
    fork_results: Mutex<VecDeque<Result<String, BackendFailure>>>,
    resume_writer_conflicts: Mutex<BTreeSet<String>>,
    starts: Mutex<Vec<(String, String)>>,
}

impl TurnBackend for FakeBackend {
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
        Box::pin(async { Ok(Vec::new()) })
    }

    fn resume_thread<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async move {
            if self
                .resume_writer_conflicts
                .lock()
                .await
                .contains(thread_id)
            {
                return Err(BackendFailure::active_writer(format!(
                    "thread/resume failed: thread {thread_id} already has an active writer"
                )));
            }
            Ok(())
        })
    }

    fn fork_thread<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.forks.lock().await.push(thread_id.to_owned());
            self.fork_results
                .lock()
                .await
                .pop_front()
                .unwrap_or_else(|| Ok(format!("fork-of-{thread_id}")))
        })
    }

    fn start_turn<'a>(
        &'a self,
        thread_id: &'a str,
        prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            let mut starts = self.starts.lock().await;
            starts.push((thread_id.to_owned(), prompt.to_owned()));
            let turn_id = format!("turn-{}", starts.len());
            drop(starts);
            self.active
                .lock()
                .await
                .insert(thread_id.to_owned(), turn_id.clone());
            Ok(turn_id)
        })
    }
}

#[tokio::test]
async fn app_server_only_handoff_quarantines_ambiguous_start_and_moves_pending_work() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "desktop-owned", "project", "Original", 70, 71, 1.0).unwrap();
    enqueue(
        &db,
        job("ambiguous", "desktop-owned", 9, 101, "do not replay", 1.0),
    )
    .unwrap();
    begin_attempt(&db, "ambiguous", &["baseline".into()], 9).unwrap();
    record_start_failure(
        &db,
        "ambiguous",
        9,
        "thread/fork predecessor start timed out",
        true,
    )
    .unwrap();
    enqueue(&db, job("pending", "desktop-owned", 8, 102, "move me", 2.0)).unwrap();

    let backend = Arc::new(FakeBackend::default());
    backend
        .fork_results
        .lock()
        .await
        .push_back(Ok("bot-owned".into()));
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let target = queue
        .ensure_app_server_only_target("desktop-owned")
        .await
        .unwrap();

    assert_eq!(target.thread_id, "bot-owned");
    assert_eq!(target.forked_from.as_deref(), Some("desktop-owned"));
    assert_eq!(target.quarantined_job_id.as_deref(), Some("ambiguous"));
    assert_eq!(*backend.forks.lock().await, vec!["desktop-owned"]);
    let jobs = list(&db).unwrap();
    assert_eq!(find(&jobs, "ambiguous").state, QueueJobState::Quarantined);
    assert_eq!(find(&jobs, "pending").target_thread_id, "bot-owned");
    assert_eq!(thread_channels(&db, "desktop-owned").unwrap(), None);
    assert_eq!(thread_channels(&db, "bot-owned").unwrap(), Some((70, 71)));
    let duplicate = queue.replay_submission_for_message(101).unwrap().unwrap();
    assert!(!duplicate.queued);
    assert_eq!(duplicate.turn_id, None);
    assert_eq!(
        duplicate.warning.unwrap().kind,
        BackendFailureKind::Quarantined
    );

    let repeated = queue
        .ensure_app_server_only_target("bot-owned")
        .await
        .unwrap();
    assert_eq!(repeated.thread_id, "bot-owned");
    assert_eq!(repeated.forked_from, None);
    assert_eq!(*backend.forks.lock().await, vec!["desktop-owned"]);
}

#[tokio::test]
async fn uncertain_fork_is_fenced_and_never_automatically_issued_twice() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "source", "project", "Original", 80, 81, 1.0).unwrap();
    enqueue(
        &db,
        job("pending", "source", 9, 177, "preserve exactly once", 1.0),
    )
    .unwrap();
    let backend = Arc::new(FakeBackend::default());
    backend
        .fork_results
        .lock()
        .await
        .push_back(Err(BackendFailure::ambiguous(
            "thread/fork response timed out",
        )));
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let first = queue
        .ensure_app_server_only_target("source")
        .await
        .unwrap_err()
        .to_string();
    let second = queue
        .ensure_app_server_only_target("source")
        .await
        .unwrap_err()
        .to_string();

    assert!(first.contains("thread/fork response timed out"));
    assert!(second.contains("unresolved"));
    assert_eq!(*backend.forks.lock().await, vec!["source"]);

    let handoff = unresolved_app_server_fork_handoff_for_source(&db, "source")
        .unwrap()
        .expect("ambiguous fork must remain durably fenced");
    assert!(handoff.fork_failure_ambiguous);
    assert_eq!(handoff.last_fork_error, "thread/fork response timed out");
    let pending = find(&list(&db).unwrap(), "pending").clone();
    assert!(pending.last_error.starts_with(UNRESOLVED_FORK_ERROR_PREFIX));
    let duplicate = queue.replay_submission_for_message(177).unwrap().unwrap();
    assert!(duplicate.queued);
    let warning = duplicate.warning.expect("fenced replay must be explicit");
    assert_eq!(warning.kind, BackendFailureKind::ForkFenced);
    assert!(warning.ambiguous);
}

#[tokio::test]
async fn concurrent_source_resolution_reuses_one_completed_fork() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "source", "project", "Original", 78, 79, 1.0).unwrap();
    let backend = Arc::new(FakeBackend::default());
    backend
        .fork_results
        .lock()
        .await
        .push_back(Ok("managed".into()));
    let queue = Arc::new(QueueCoordinator::new(db, Arc::clone(&backend)));

    let (first, second) = tokio::join!(
        queue.ensure_app_server_only_target("source"),
        queue.ensure_app_server_only_target("source")
    );

    assert_eq!(first.unwrap().thread_id, "managed");
    assert_eq!(second.unwrap().thread_id, "managed");
    assert_eq!(*backend.forks.lock().await, vec!["source"]);
}

#[tokio::test]
async fn definite_fork_rejection_releases_the_fence_for_one_safe_later_retry() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "source", "project", "Original", 82, 83, 1.0).unwrap();
    let backend = Arc::new(FakeBackend::default());
    backend
        .fork_results
        .lock()
        .await
        .push_back(Err(BackendFailure::definite("thread/fork rejected")));
    backend
        .fork_results
        .lock()
        .await
        .push_back(Ok("managed".into()));
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let first = queue.ensure_app_server_only_target("source").await;
    assert!(
        first
            .unwrap_err()
            .to_string()
            .contains("thread/fork rejected")
    );
    assert!(
        cdr_store::queue::unresolved_app_server_fork_handoff_for_source(&db, "source")
            .unwrap()
            .is_none()
    );

    let retried = queue.ensure_app_server_only_target("source").await.unwrap();
    assert_eq!(retried.thread_id, "managed");
    assert_eq!(*backend.forks.lock().await, vec!["source", "source"]);
}

#[tokio::test]
async fn a_fork_target_is_visible_when_durable_staging_fails() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "source", "project", "Original", 182, 183, 1.0).unwrap();
    let backend = Arc::new(FakeBackend::default());
    backend
        .fork_results
        .lock()
        .await
        .push_back(Ok(" target-with-invalid-whitespace ".into()));
    let queue = QueueCoordinator::new(db, Arc::clone(&backend));

    let error = queue
        .ensure_app_server_only_target("source")
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("target-with-invalid-whitespace"));
    assert!(error.contains("durably stage"));
}

#[tokio::test]
async fn restart_finalizes_an_observed_fork_target_without_a_second_rpc() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "source", "project", "Original", 84, 85, 1.0).unwrap();
    enqueue(&db, job("pending", "source", 9, 149, "preserve", 1.0)).unwrap();
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "crash-window",
            ambiguous_job_id: None,
            source_thread_id: "source",
            expected_generation: 9,
            quarantine_reason: "proactive",
        },
    )
    .unwrap();
    stage_app_server_fork_target(&db, "crash-window", "observed-target").unwrap();
    let backend = Arc::new(FakeBackend::default());
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let recovered = queue.ensure_app_server_only_target("source").await.unwrap();

    assert_eq!(recovered.thread_id, "observed-target");
    assert!(backend.forks.lock().await.is_empty());
    assert_eq!(
        find(&list(&db).unwrap(), "pending").target_thread_id,
        "observed-target"
    );
}

#[tokio::test]
async fn a_managed_source_still_finalizes_its_newer_observed_fork() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "original", "project", "Original", 186, 187, 1.0).unwrap();
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "first",
            ambiguous_job_id: None,
            source_thread_id: "original",
            expected_generation: 9,
            quarantine_reason: "first ownership fork",
        },
    )
    .unwrap();
    complete_app_server_fork_handoff(&db, "first", "managed-b", 9).unwrap();
    enqueue(&db, job("pending", "managed-b", 9, 189, "move again", 2.0)).unwrap();
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "second",
            ambiguous_job_id: None,
            source_thread_id: "managed-b",
            expected_generation: 9,
            quarantine_reason: "second ownership fork",
        },
    )
    .unwrap();
    stage_app_server_fork_target(&db, "second", "managed-c").unwrap();
    let backend = Arc::new(FakeBackend::default());
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let target = queue
        .ensure_app_server_only_target("managed-b")
        .await
        .unwrap();

    assert_eq!(target.thread_id, "managed-c");
    assert!(backend.forks.lock().await.is_empty());
    assert_eq!(
        find(&list(&db).unwrap(), "pending").target_thread_id,
        "managed-c"
    );
}

#[tokio::test]
async fn a_managed_source_never_bypasses_its_ambiguous_newer_fork_fence() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "original", "project", "Original", 190, 191, 1.0).unwrap();
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "first",
            ambiguous_job_id: None,
            source_thread_id: "original",
            expected_generation: 9,
            quarantine_reason: "first ownership fork",
        },
    )
    .unwrap();
    complete_app_server_fork_handoff(&db, "first", "managed-b", 9).unwrap();
    enqueue(
        &db,
        job("pending", "managed-b", 9, 192, "do not start", 2.0),
    )
    .unwrap();
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "second",
            ambiguous_job_id: None,
            source_thread_id: "managed-b",
            expected_generation: 9,
            quarantine_reason: "second ownership fork",
        },
    )
    .unwrap();
    record_app_server_fork_failure(&db, "second", "thread/fork timed out", true).unwrap();
    let backend = Arc::new(FakeBackend::default());
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let error = queue
        .ensure_app_server_only_target("managed-b")
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("unresolved"));
    assert!(backend.forks.lock().await.is_empty());
    let replay = queue.replay_submission_for_message(192).unwrap().unwrap();
    assert_eq!(replay.warning.unwrap().kind, BackendFailureKind::ForkFenced);
}

#[tokio::test]
async fn startup_recovery_finalizes_a_newer_staged_fork_of_a_managed_target() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "original", "project", "Original", 193, 194, 1.0).unwrap();
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "first",
            ambiguous_job_id: None,
            source_thread_id: "original",
            expected_generation: 9,
            quarantine_reason: "first ownership fork",
        },
    )
    .unwrap();
    complete_app_server_fork_handoff(&db, "first", "managed-b", 9).unwrap();
    enqueue(&db, job("pending", "managed-b", 9, 195, "start on c", 2.0)).unwrap();
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "second",
            ambiguous_job_id: None,
            source_thread_id: "managed-b",
            expected_generation: 9,
            quarantine_reason: "second ownership fork",
        },
    )
    .unwrap();
    stage_app_server_fork_target(&db, "second", "managed-c").unwrap();
    let backend = Arc::new(FakeBackend::default());
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    queue.recover().await.unwrap();

    assert!(backend.forks.lock().await.is_empty());
    assert_eq!(
        *backend.starts.lock().await,
        vec![("managed-c".into(), "start on c".into())]
    );
    let jobs = list(&db).unwrap();
    let pending = find(&jobs, "pending");
    assert_eq!(pending.target_thread_id, "managed-c");
    assert_eq!(pending.state, QueueJobState::Running);
}

#[tokio::test]
async fn startup_recovery_finalizes_a_staged_ambiguous_start_before_replay() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "source", "project", "Original", 201, 202, 1.0).unwrap();
    enqueue(&db, job("ambiguous", "source", 9, 203, "never replay", 1.0)).unwrap();
    begin_attempt(&db, "ambiguous", &[], 9).unwrap();
    record_start_failure(&db, "ambiguous", 9, "turn/start timed out", true).unwrap();
    enqueue(&db, job("pending", "source", 9, 204, "continue once", 2.0)).unwrap();
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "staged-ambiguous",
            ambiguous_job_id: Some("ambiguous"),
            source_thread_id: "source",
            expected_generation: 9,
            quarantine_reason: "recover ambiguous start",
        },
    )
    .unwrap();
    stage_app_server_fork_target(&db, "staged-ambiguous", "managed").unwrap();
    let backend = Arc::new(FakeBackend::default());
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    queue.recover().await.unwrap();

    assert!(backend.forks.lock().await.is_empty());
    assert_eq!(
        *backend.starts.lock().await,
        vec![("managed".into(), "continue once".into())]
    );
    let jobs = list(&db).unwrap();
    assert_eq!(find(&jobs, "ambiguous").state, QueueJobState::Quarantined);
    assert_eq!(find(&jobs, "pending").target_thread_id, "managed");
    assert_eq!(find(&jobs, "pending").state, QueueJobState::Running);
}

#[tokio::test]
async fn restart_marks_an_unanswered_fork_intent_as_interrupted_without_reissuing_it() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "source", "project", "Original", 184, 185, 1.0).unwrap();
    enqueue(&db, job("pending", "source", 9, 188, "preserve", 1.0)).unwrap();
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "interrupted-before-response",
            ambiguous_job_id: None,
            source_thread_id: "source",
            expected_generation: 9,
            quarantine_reason: "proactive",
        },
    )
    .unwrap();
    let backend = Arc::new(FakeBackend::default());
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let error = queue
        .ensure_app_server_only_target("source")
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("interrupted before a response"));
    assert!(backend.forks.lock().await.is_empty());
    let handoff = unresolved_app_server_fork_handoff_for_source(&db, "source")
        .unwrap()
        .unwrap();
    assert!(handoff.fork_failure_ambiguous);
    assert!(
        handoff
            .last_fork_error
            .contains("interrupted before a response")
    );
    assert!(
        find(&list(&db).unwrap(), "pending")
            .last_error
            .starts_with(UNRESOLVED_FORK_ERROR_PREFIX)
    );
}

#[tokio::test]
async fn a_staged_target_with_mapping_drift_records_a_visible_finalize_failure() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "source", "project", "Original", 196, 197, 1.0).unwrap();
    enqueue(&db, job("pending", "source", 9, 198, "keep fenced", 1.0)).unwrap();
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "mapping-drift",
            ambiguous_job_id: None,
            source_thread_id: "source",
            expected_generation: 9,
            quarantine_reason: "proactive",
        },
    )
    .unwrap();
    stage_app_server_fork_target(&db, "mapping-drift", "observed-target").unwrap();
    upsert_thread(&db, "mapping-racer", "project", "Other", 299, 197, 2.0).unwrap();
    let backend = Arc::new(FakeBackend::default());
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let error = queue
        .ensure_app_server_only_target("source")
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("observed-target"));
    assert!(error.contains("could not be finalized"));
    let handoff = unresolved_app_server_fork_handoff_for_source(&db, "source")
        .unwrap()
        .unwrap();
    assert_eq!(
        handoff.observed_target_thread_id.as_deref(),
        Some("observed-target")
    );
    assert!(handoff.last_fork_error.contains("mapping"));
    assert!(
        find(&list(&db).unwrap(), "pending")
            .last_error
            .starts_with(UNRESOLVED_FORK_ERROR_PREFIX)
    );
    assert!(backend.forks.lock().await.is_empty());
}

#[tokio::test]
async fn uncertain_startup_fork_keeps_recovery_alive_but_never_reissues_the_rpc() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "source", "project", "Original", 85, 86, 1.0).unwrap();
    enqueue(&db, job("pending", "source", 9, 151, "preserve", 1.0)).unwrap();
    let backend = Arc::new(FakeBackend::default());
    backend
        .fork_results
        .lock()
        .await
        .push_back(Err(BackendFailure::ambiguous(
            "thread/fork response timed out",
        )));
    let queue = QueueCoordinator::new(db, Arc::clone(&backend));

    let first = queue.recover().await.unwrap();
    let second = queue.recover().await.unwrap();

    assert!(first.unavailable_targets.contains("source"));
    assert!(second.unavailable_targets.contains("source"));
    assert_eq!(*backend.forks.lock().await, vec!["source"]);
}

#[tokio::test]
async fn recovery_proactively_forks_an_unmanaged_mirror_before_starting_its_pending_prompt() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "source", "project", "Original", 90, 91, 1.0).unwrap();
    enqueue(
        &db,
        job("pending", "source", 7, 201, "continue safely", 1.0),
    )
    .unwrap();
    let backend = Arc::new(FakeBackend::default());
    backend
        .fork_results
        .lock()
        .await
        .push_back(Ok("managed".into()));
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    queue.recover().await.unwrap();

    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].target_thread_id, "managed");
    assert_eq!(jobs[0].state, QueueJobState::Running);
    assert_eq!(
        *backend.starts.lock().await,
        vec![("managed".into(), "continue safely".into())]
    );
}

#[tokio::test]
async fn recovery_moves_pending_work_again_if_a_managed_target_gets_a_desktop_writer() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "initial", "project", "Original", 100, 101, 1.0).unwrap();
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "seed",
            ambiguous_job_id: None,
            source_thread_id: "initial",
            expected_generation: 9,
            quarantine_reason: "seed managed target",
        },
    )
    .unwrap();
    complete_app_server_fork_handoff(&db, "seed", "managed", 9).unwrap();
    enqueue(&db, job("pending", "managed", 9, 301, "keep moving", 2.0)).unwrap();
    let backend = Arc::new(FakeBackend::default());
    backend
        .resume_writer_conflicts
        .lock()
        .await
        .insert("managed".into());
    backend
        .fork_results
        .lock()
        .await
        .push_back(Ok("managed-2".into()));
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    queue.recover().await.unwrap();

    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].target_thread_id, "managed-2");
    assert_eq!(jobs[0].state, QueueJobState::Running);
    assert_eq!(*backend.forks.lock().await, vec!["managed"]);
}

fn job<'a>(
    id: &'a str,
    target: &'a str,
    generation: i64,
    message: i64,
    prompt: &'a str,
    created_at: f64,
) -> NewQueueJob<'a> {
    NewQueueJob {
        job_id: id,
        target_thread_id: target,
        channel_id: 71,
        owner_user_id: Some(10),
        discord_message_id: Some(message),
        app_server_generation: generation,
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
