use super::*;
use crate::{client::WriteTestPause, idle_release::IdleReleaseJournal};

struct Journal;
impl IdleReleaseJournal for Journal {
    fn before_mutation(
        &self,
        _: &str,
        _: u64,
        _: &str,
    ) -> Result<Option<IdleReleaseToken>, AppServerError> {
        panic!("not a normal mutation fixture")
    }
    fn check_mutation(&self, _: &str) -> Result<(), AppServerError> {
        panic!("not a normal mutation fixture")
    }
    fn resume_required(&self, _: &str) -> Result<bool, AppServerError> {
        panic!("not a resume fixture")
    }
    fn verify(&self, t: &IdleReleaseToken, _: bool) -> Result<(), AppServerError> {
        assert_eq!(t.state, "Dispatching");
        Ok(())
    }
    fn transition(
        &self,
        _: &IdleReleaseToken,
        _: &str,
        _: &str,
    ) -> Result<IdleReleaseToken, AppServerError> {
        panic!("transport fixture does not settle effects")
    }
    fn old_child_exited(&self, _: &str, _: u64) -> Result<(), AppServerError> {
        Ok(())
    }
}

fn prepared() -> (Arc<ResidentAppServer>, crate::AppServerClient, Work) {
    let (server, client) = super::super::close_retry_tests::test_server();
    let server = Arc::new(server);
    client.inner.state.lock().unwrap().initialized = true;
    server
        .install_idle_release_journal(Arc::new(Journal))
        .unwrap();
    let token = IdleReleaseToken {
        intent_id: "i".into(),
        owner_id: server.instance_id().into(),
        generation: 1,
        thread_id: "A".into(),
        turn_id: "T1".into(),
        job_id: "j".into(),
        revision: 2,
        state: "Dispatching".into(),
        detail: String::new(),
    };
    let permit = server.target_gate.reserve(&token).unwrap();
    let work = server.idle_work(permit, token).unwrap();
    (server, client, work)
}

#[tokio::test]
async fn ir7_flushed_maintenance_timeout_does_not_quarantine_shared_transport() {
    let (server, _client, work) = prepared();
    let (result, phase) = work
        .rpc(
            "thread/unsubscribe",
            json!({"threadId":"A"}),
            Duration::from_millis(30),
            false,
            None,
        )
        .await;
    assert!(matches!(result, Err(AppServerError::Timeout { .. })));
    assert!(phase == transport::WritePhase::Flushed);
    assert!(server.lifecycle_snapshot().await.healthy);
    assert!(!server.lifecycle_snapshot().await.quarantined);
    drop(work);
    server.close().await.unwrap();
}

#[tokio::test]
async fn ir7_partial_maintenance_write_preserves_global_transport_safety() {
    let (server, client, work) = prepared();
    *client.inner.write_pause.lock().unwrap() = Some(Arc::new(WriteTestPause::failing()));
    let (result, phase) = work
        .rpc(
            "thread/unsubscribe",
            json!({"threadId":"A"}),
            Duration::from_secs(1),
            false,
            None,
        )
        .await;
    assert!(matches!(result, Err(AppServerError::Io(_))));
    assert!(phase == transport::WritePhase::Partial);
    let status = server.lifecycle_snapshot().await;
    assert!(status.quarantined && status.restart_pending && !status.healthy);
    drop(work);
    server.close().await.unwrap();
}

#[tokio::test]
async fn ir7_expired_deadline_while_waiting_for_writer_proves_no_send() {
    let (server, client, work) = prepared();
    let pause = Arc::new(WriteTestPause::for_response_resolution());
    *client.inner.write_pause.lock().unwrap() = Some(Arc::clone(&pause));
    let writer = client.inner.stdin.lock().await;
    let task = tokio::spawn(async move {
        work.rpc(
            "thread/unsubscribe",
            json!({"threadId":"A"}),
            Duration::from_millis(20),
            false,
            None,
        )
        .await
    });
    pause.wait_until_before_lock().await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    drop(writer);
    let (result, phase) = task.await.unwrap();
    assert!(matches!(result, Err(AppServerError::Timeout { .. })));
    assert!(phase == transport::WritePhase::NotStarted);
    assert!(!server.lifecycle_snapshot().await.quarantined);
    server.close().await.unwrap();
}
