//! The discovery cursor must not discard metadata just because ready work is full.
use super::*;
use crate::app_backend::AppServerTurnBackend;
use crate::queue_runner::QueueCoordinator;
use crate::restart_readiness::drain::AdmissionGate;
use cdr_app_server::ResidentAppServer;
use std::sync::Arc;

#[tokio::test]
async fn full_ready_queue_does_not_lose_late_orphans_from_a_finite_pass() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.sqlite");
    let mut db = cdr_store::schema::open_initialized(&path).unwrap();
    let tx = db.transaction().unwrap();
    for i in 0..160 {
        tx.execute(
            "INSERT INTO cdr_async_execution_obligations
             (question_id,thread_id,origin_job_id,turn_id,channel_id,format_version,revision,
              answer_state,execution_state,admission_state,policy,original_seal,claim_json,
              owner_json,original_error,created_at,updated_at)
             VALUES(?1,?2,?3,'original',20,1,0,'unresolved','unresolved','held','ordinary',
                    NULL,'{}',NULL,'original uncertainty',1,1)",
            rusqlite::params![
                format!("question-{i:03}"),
                format!("orphan-{i:03}"),
                format!("lost-{i:03}")
            ],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let mut config = crate::test_support::native_fixture::config("async-question");
    config.environment.insert(
        "CDR_ACTION_RPC_LOG".into(),
        temp.path().join("rpc.jsonl").to_string_lossy().into_owned(),
    );
    let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
    let worker = CompletionWorker {
        server: server.clone(),
        queue: Arc::new(QueueCoordinator::new_with_admission_gate(
            path,
            Arc::new(AppServerTurnBackend::new(server.clone())),
            AdmissionGate::new(),
        )),
        http: Arc::new(
            twilight_http::Client::builder()
                .token("fixture-token".into())
                .build(),
        ),
        commentary_enabled: false,
        history_read_timeout: Duration::from_secs(2),
        commentary: tokio::sync::Mutex::new(crate::commentary_stream::CommentaryBuffer::default()),
        terminal_fence: super::super::terminal_fence::TerminalFence::default(),
    };
    let mut scans = [Scan {
        source: Source::AsyncOrphan,
        cursor: Cursor::default(),
        next: Instant::now(),
        wake: false,
        pending: VecDeque::new(),
    }];
    let mut next = 0;
    let mut ready = Ready::default();
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..512 {
        discover_round(&mut scans, &mut next, &worker, &mut ready, &HashSet::new());
        assert!(
            ready.state.len() <= lanes::READY_CAP,
            "ready capacity was increased instead of respecting backpressure"
        );
        if let Some(StateWork::Durable(entry)) = ready.state.pop_front() {
            seen.insert(entry.target);
        }
    }
    server.close().await.unwrap();
    assert_eq!(
        seen.len(),
        160,
        "a full ready queue silently discarded the tail of the finite discovery pass"
    );
    assert!(seen.contains("orphan-159"));
}
