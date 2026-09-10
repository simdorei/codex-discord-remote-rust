use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use cdr_runtime::action_executor::{ActionContext, ActionExecutor};
use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::command_plan::CommandAction;
use cdr_store::prompt_intake::{
    NewPromptIntake, admit_prompt_intake, list_prompt_intakes, try_claim_prompt_intake,
};
use cdr_store::queue::{
    NewAppServerForkHandoff, NewQueueJob, begin_app_server_fork_handoff,
    complete_app_server_fork_handoff, enqueue, list, mark_app_server_managed_target,
    stage_app_server_fork_target,
};

#[path = "support/prompt_intake.rs"]
mod support;
use support::{ForkGate, IntakeBackend, RecordingPreprocessor, executor};

#[tokio::test]
async fn intake_is_durable_and_non_startable_while_fork_is_in_flight() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("source")).unwrap();
    let gate = Arc::new(ForkGate::new(db.clone()));
    let backend = Arc::new(IntakeBackend {
        fork_targets: BTreeMap::from([("source".into(), "destination".into())]),
        fork_gate: Some(Arc::clone(&gate)),
        ..IntakeBackend::default()
    });
    let (queue, executor) = executor(
        &temp,
        db.clone(),
        bridge,
        Arc::clone(&backend),
        Arc::new(RecordingPreprocessor::new()),
    );
    let running = tokio::spawn({
        let executor = Arc::clone(&executor);
        async move {
            executor
                .execute_with_context(
                    CommandAction::Ask {
                        prompt: "never lose me".into(),
                    },
                    context(501),
                )
                .await
        }
    });

    gate.entered.acquire().await.unwrap().forget();
    assert_eq!(gate.observed_intakes.load(Ordering::SeqCst), 1);
    assert_eq!(gate.observed_queue_jobs.load(Ordering::SeqCst), 0);
    let report = queue.recover().await.unwrap();
    let status = executor
        .execute(CommandAction::Runners, 99, 20)
        .await
        .unwrap();

    assert_eq!(report, cdr_runtime::queue_runner::RecoveryReport::default());
    assert!(list(&db).unwrap().is_empty());
    assert!(backend.starts.lock().await.is_empty());
    assert_eq!(list_prompt_intakes(&db).unwrap().len(), 1);
    assert!(status.text.contains("pending: 0"));
    assert!(status.text.contains("recoverable_intakes: 1"));
    running.abort();
}

#[tokio::test]
async fn restart_finalizes_a_staged_target_then_prepares_and_starts_only_there() {
    let fixture = Fixture::new("source", "staged-target", 601);
    begin_handoff(&fixture.db, "handoff-staged", "source");
    stage_app_server_fork_target(&fixture.db, "handoff-staged", "staged-target").unwrap();

    assert_eq!(fixture.executor.recover_prompt_intakes().await.unwrap(), 1);

    fixture
        .assert_recovered("staged-target", "prepared:staged-target:raw")
        .await;
    assert!(fixture.backend.forks.lock().await.is_empty());
}

#[tokio::test]
async fn restart_follows_an_already_finalized_target_and_reprepares_for_it() {
    let fixture = Fixture::new("source", "final-target", 602);
    begin_handoff(&fixture.db, "handoff-final", "source");
    complete_app_server_fork_handoff(&fixture.db, "handoff-final", "final-target", 7).unwrap();

    assert_eq!(fixture.executor.recover_prompt_intakes().await.unwrap(), 1);

    fixture
        .assert_recovered("final-target", "prepared:final-target:raw")
        .await;
    assert!(fixture.backend.forks.lock().await.is_empty());
}

#[tokio::test]
async fn restart_cleans_post_enqueue_intake_and_starts_the_stable_job_once() {
    let fixture = Fixture::new("managed", "unused", 603);
    mark_app_server_managed_target(&fixture.db, "managed", 7).unwrap();
    let intake = list_prompt_intakes(&fixture.db).unwrap().remove(0);
    try_claim_prompt_intake(&fixture.db, &intake.job_id, now(), now() + 600.0)
        .unwrap()
        .unwrap();
    enqueue(
        &fixture.db,
        NewQueueJob {
            job_id: &intake.job_id,
            target_thread_id: "managed",
            channel_id: 99,
            owner_user_id: Some(20),
            discord_message_id: Some(603),
            app_server_generation: 7,
            prompt: "prepared:managed:raw",
            queued: false,
            ack_sent: true,
            created_at: now(),
        },
    )
    .unwrap();

    fixture.queue.recover().await.unwrap();
    assert_eq!(
        fixture
            .executor
            .recover_prompt_intakes_on_startup()
            .await
            .unwrap(),
        0
    );
    assert_eq!(fixture.executor.recover_prompt_intakes().await.unwrap(), 0);

    assert!(list_prompt_intakes(&fixture.db).unwrap().is_empty());
    assert_eq!(list(&fixture.db).unwrap().len(), 1);
    assert_eq!(fixture.backend.starts.lock().await.len(), 1);
}

struct Fixture {
    _temp: tempfile::TempDir,
    db: std::path::PathBuf,
    queue: Arc<cdr_runtime::queue_runner::QueueCoordinator<IntakeBackend>>,
    executor: Arc<ActionExecutor<IntakeBackend>>,
    backend: Arc<IntakeBackend>,
}

impl Fixture {
    fn new(source: &str, target: &str, message_id: i64) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        admit(&db, source, message_id);
        let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
        bridge.set_selected_thread_id(Some(source)).unwrap();
        let backend = Arc::new(IntakeBackend {
            fork_targets: BTreeMap::from([(source.into(), target.into())]),
            ..IntakeBackend::default()
        });
        let (queue, executor) = executor(
            &temp,
            db.clone(),
            bridge,
            Arc::clone(&backend),
            Arc::new(RecordingPreprocessor::new()),
        );
        Self {
            _temp: temp,
            db,
            queue,
            executor,
            backend,
        }
    }

    async fn assert_recovered(&self, target: &str, prompt: &str) {
        assert!(list_prompt_intakes(&self.db).unwrap().is_empty());
        let jobs = list(&self.db).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].target_thread_id, target);
        assert_eq!(jobs[0].prompt, prompt);
        assert_eq!(
            self.backend.starts.lock().await.as_slice(),
            &[(target.into(), prompt.into())]
        );
    }
}

fn admit(db: &std::path::Path, target: &str, message_id: i64) {
    admit_prompt_intake(
        db,
        NewPromptIntake {
            job_id: &format!("job-{message_id}"),
            target_thread_id: target,
            channel_id: 99,
            owner_user_id: Some(20),
            discord_message_id: Some(message_id),
            raw_prompt: "raw",
            auto_queue_when_busy: true,
            require_current_mirror: false,
            created_at: now(),
        },
    )
    .unwrap();
}

fn begin_handoff(db: &std::path::Path, handoff_id: &str, source: &str) {
    begin_app_server_fork_handoff(
        db,
        NewAppServerForkHandoff {
            handoff_id,
            ambiguous_job_id: None,
            source_thread_id: source,
            expected_generation: 7,
            quarantine_reason: "prompt intake crash contract",
        },
    )
    .unwrap();
}

fn context(message_id: u64) -> ActionContext {
    ActionContext {
        channel_id: 99,
        user_id: 20,
        discord_message_id: Some(message_id),
        auto_queue_when_busy: true,
    }
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
