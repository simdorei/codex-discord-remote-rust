use std::sync::Arc;

use cdr_runtime::queue_runner::{
    BackendFailure, BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord,
};
use cdr_store::mapping::thread_channels;
use cdr_store::queue::{
    NewAppServerForkHandoff, NewQueueJob, QueueJobState, begin_app_server_fork_handoff,
    complete_app_server_fork_handoff, enqueue, list,
};
use tokio::sync::Mutex;

#[derive(Default)]
struct UnmappedBackend {
    forks: Mutex<Vec<String>>,
    resumes: Mutex<Vec<String>>,
    starts: Mutex<Vec<(String, String)>>,
}

impl TurnBackend for UnmappedBackend {
    fn generation(&self) -> u64 {
        9
    }

    fn requires_app_server_fork(&self) -> bool {
        true
    }

    fn active_turn_id<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }

    fn read_turns<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn resume_thread<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async move {
            self.resumes.lock().await.push(thread_id.to_owned());
            if matches!(thread_id, "python-era" | "managed-unmapped") {
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
            Ok(format!("app-server-{thread_id}"))
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
                .push((thread_id.to_owned(), prompt.to_owned()));
            Ok(format!("turn-{thread_id}"))
        })
    }
}

#[tokio::test]
async fn python_era_unmapped_pending_is_forked_before_resume_and_started_once() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    enqueue(&db, job("python-era", "run once")).unwrap();
    let backend = Arc::new(UnmappedBackend::default());
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    queue.recover().await.unwrap();
    queue.recover().await.unwrap();

    assert_eq!(*backend.forks.lock().await, vec!["python-era"]);
    assert_eq!(*backend.resumes.lock().await, vec!["app-server-python-era"]);
    assert_eq!(
        *backend.starts.lock().await,
        vec![("app-server-python-era".into(), "run once".into())]
    );
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].target_thread_id, "app-server-python-era");
    assert_eq!(jobs[0].state, QueueJobState::Running);
    assert_eq!(
        jobs[0].turn_id.as_deref(),
        Some("turn-app-server-python-era")
    );
    assert_eq!(thread_channels(&db, "python-era").unwrap(), None);
    assert_eq!(thread_channels(&db, "app-server-python-era").unwrap(), None);
}

#[tokio::test]
async fn unmapped_managed_target_with_a_new_desktop_writer_uses_safe_fallback_fork() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "seed",
            ambiguous_job_id: None,
            source_thread_id: "seed-source",
            expected_generation: 9,
            quarantine_reason: "seed unmapped app-server target",
        },
    )
    .unwrap();
    complete_app_server_fork_handoff(&db, "seed", "managed-unmapped", 9).unwrap();
    enqueue(&db, job("managed-unmapped", "continue once")).unwrap();
    let backend = Arc::new(UnmappedBackend::default());
    let queue = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    queue.recover().await.unwrap();

    assert_eq!(*backend.forks.lock().await, vec!["managed-unmapped"]);
    assert_eq!(
        *backend.resumes.lock().await,
        vec!["managed-unmapped", "app-server-managed-unmapped"]
    );
    assert_eq!(
        *backend.starts.lock().await,
        vec![("app-server-managed-unmapped".into(), "continue once".into())]
    );
    let jobs = list(&db).unwrap();
    let continued = jobs
        .iter()
        .find(|job| job.prompt == "continue once")
        .unwrap();
    assert_eq!(continued.target_thread_id, "app-server-managed-unmapped");
    assert_eq!(continued.state, QueueJobState::Running);
}

fn job<'a>(target: &'a str, prompt: &'a str) -> NewQueueJob<'a> {
    NewQueueJob {
        job_id: prompt,
        target_thread_id: target,
        channel_id: 70,
        owner_user_id: Some(10),
        discord_message_id: None,
        app_server_generation: 9,
        prompt,
        queued: true,
        ack_sent: true,
        created_at: 2.0,
    }
}
