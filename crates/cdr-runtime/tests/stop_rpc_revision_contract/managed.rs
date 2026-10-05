use super::*;
use cdr_app_server::idle_release::{IdleReleaseJournal, IdleReleaseToken};
use std::sync::Mutex;

struct ResumeJournal {
    db: PathBuf,
    token: Mutex<IdleReleaseToken>,
    stop_once: AtomicBool,
}

impl IdleReleaseJournal for ResumeJournal {
    fn before_mutation(
        &self,
        owner: &str,
        generation: u64,
        target: &str,
    ) -> Result<Option<IdleReleaseToken>, AppServerError> {
        if target != "thread-b" {
            return Ok(None);
        }
        let mut token = self.token.lock().unwrap();
        assert_eq!(token.owner_id, owner);
        assert_eq!(token.generation, generation);
        if token.state == "Settled" {
            return Ok(None);
        }
        if self.stop_once.swap(false, Ordering::AcqRel) {
            accept_nonrunning(&self.db,StopScope{target,channel:42,owner:3},
                &json!({"target":target,"route":"Explicit","command":{"Stop":{"reference":target}}}),
                None,|| Ok(())).unwrap().unwrap();
        }
        assert_eq!(token.state, "AwaitUnload");
        token.state = "Resubscribing".into();
        token.revision += 1;
        Ok(Some(token.clone()))
    }
    fn check_mutation(&self, target: &str) -> Result<(), AppServerError> {
        if target == "thread-b" {
            assert_eq!(self.token.lock().unwrap().state, "Settled");
        }
        Ok(())
    }
    fn resume_required(&self, target: &str) -> Result<bool, AppServerError> {
        Ok(target == "thread-b" && self.token.lock().unwrap().state != "Settled")
    }
    fn verify(&self, token: &IdleReleaseToken, _idle: bool) -> Result<(), AppServerError> {
        assert_eq!(*self.token.lock().unwrap(), *token);
        Ok(())
    }
    fn transition(
        &self,
        original: &IdleReleaseToken,
        state: &str,
        detail: &str,
    ) -> Result<IdleReleaseToken, AppServerError> {
        let mut token = self.token.lock().unwrap();
        assert_eq!(*token, *original);
        token.state = state.into();
        token.detail = detail.into();
        token.revision += 1;
        Ok(token.clone())
    }
    fn old_child_exited(&self, _owner: &str, _generation: u64) -> Result<(), AppServerError> {
        Ok(())
    }
}

fn resume_journal(db: &std::path::Path, server: &ResidentAppServer) -> Arc<ResumeJournal> {
    queue::enqueue(
        db,
        NewQueueJob {
            job_id: "original",
            target_thread_id: "thread-b",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: Some(801),
            app_server_generation: i64::try_from(server.generation()).unwrap(),
            prompt: "original A",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    Arc::new(ResumeJournal {
        db: db.to_path_buf(),
        stop_once: AtomicBool::new(true),
        token: Mutex::new(IdleReleaseToken {
            intent_id: "test-idle".into(),
            owner_id: server.instance_id().into(),
            generation: server.generation(),
            thread_id: "thread-b".into(),
            turn_id: "old-turn".into(),
            job_id: "old-job".into(),
            revision: 1,
            state: "AwaitUnload".into(),
            detail: String::new(),
        }),
    })
}

#[tokio::test]
async fn managed_resume_keeps_the_parent_rpc_origin_and_fresh_requests_are_distinct() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("store.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let fence = Arc::new(
        RuntimeDeadGenerationFence::new(db.clone(), "managed-origin".into(), None).unwrap(),
    );
    let mut config = native_fixture::config("action");
    config.environment.insert(
        "CDR_ACTION_RPC_LOG".into(),
        log.to_string_lossy().into_owned(),
    );
    let server = Arc::new(
        ResidentAppServer::start_with_dead_generation_fence(config, fence)
            .await
            .unwrap(),
    );
    let journal = resume_journal(&db, &server);
    server
        .install_idle_release_journal(journal.clone())
        .unwrap();
    let request = || AppRequest {
        method: "thread/settings/update",
        params: json!({"threadId":"thread-b","model":"gpt-5.4","reasoningEffort":"high","serviceTier":"default"}),
        timeout: Duration::from_secs(2),
    };
    let original = server.execute(request(), None).await;
    server
        .execute(
            AppRequest {
                method: "thread/read",
                params: json!({"threadId":"thread-b"}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
    let old_calls = server_support::rpc_log(&log);
    let old_resumes = old_calls
        .iter()
        .filter(|r| r["method"] == "thread/resume")
        .count();
    let old_updates = old_calls
        .iter()
        .filter(|r| r["method"] == "thread/settings/update")
        .count();
    let coordinator = QueueCoordinator::new(
        db.clone(),
        Arc::new(AppServerTurnBackend::new(server.clone())),
    );
    let independent = tokio::time::timeout(
        Duration::from_secs(5),
        coordinator.submit("thread-c", 43, 4, Some(802), "independent B"),
    )
    .await
    .unwrap()
    .unwrap();
    let fresh = server.execute(request(), None).await;
    let all = server_support::rpc_log(&log);
    let resumes = all
        .iter()
        .filter(|r| r["method"] == "thread/resume" && r["params"]["threadId"] == "thread-b")
        .count();
    let updates = all
        .iter()
        .filter(|r| r["method"] == "thread/settings/update")
        .count();
    let a_starts = all
        .iter()
        .filter(|r| r["method"] == "turn/start" && r["params"]["threadId"] == "thread-b")
        .count();
    let healthy = !server.lifecycle_snapshot().await.quarantined;
    server.close().await.unwrap();
    assert!(original.is_err());
    assert_eq!(
        old_resumes, 0,
        "managed resume must not lose the parent's pre-stop snapshot"
    );
    assert_eq!(old_updates, 0);
    assert!(independent.turn_id.is_some());
    assert!(fresh.is_ok());
    assert_eq!(resumes, 1, "only the distinct new request can resume");
    assert_eq!(updates, 1);
    assert_eq!(a_starts, 0);
    assert!(healthy);
    assert!(
        cdr_store::execution_hold::reason(&db, "original")
            .unwrap()
            .is_some()
    );
}
