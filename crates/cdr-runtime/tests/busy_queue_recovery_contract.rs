use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cdr_runtime::bridge_state::BridgeState;
use cdr_runtime::component_worker::{busy_ready_marker, confirmation_ready};
use cdr_runtime::prompt_preprocessor::{BoxPromptFuture, PromptPreprocessor};
use cdr_store::claims::{NewBusyChoice, create_busy_choice, get_busy_choice};
use cdr_store::prompt_intake::list_prompt_intakes;
use cdr_store::queue::{QueueJobState, list, mark_app_server_managed_target};
use tokio::sync::Semaphore;

#[path = "support/prompt_intake.rs"]
mod support;
use support::{IntakeBackend, RecordingPreprocessor};

struct PausedPreparation {
    entered: Semaphore,
    calls: AtomicUsize,
}

impl PromptPreprocessor for PausedPreparation {
    fn prepare<'a>(&'a self, _prompt: &'a str, _thread_id: &'a str) -> BoxPromptFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered.add_permits(1);
            std::future::pending().await
        })
    }
}

#[tokio::test]
async fn cancelled_busy_preparation_recovers_original_request_once_after_restart() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("managed")).unwrap();
    mark_app_server_managed_target(&db, "managed", 7).unwrap();
    let backend = Arc::new(IntakeBackend::default());
    backend
        .active
        .lock()
        .await
        .insert("managed".into(), "live-turn".into());
    let paused = Arc::new(PausedPreparation {
        entered: Semaphore::new(0),
        calls: AtomicUsize::new(0),
    });
    let (_, first) = support::executor(
        &temp,
        db.clone(),
        Arc::clone(&bridge),
        Arc::clone(&backend),
        paused.clone(),
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let id = create_busy_choice(
        &db,
        NewBusyChoice {
            owner_user_id: 20,
            channel_id: 99,
            target_thread_id: Some("managed"),
            prompt: "keep this exact request",
            allow_steer: true,
            now,
            time_to_live: 1_800.0,
        },
    )
    .unwrap();
    let choice = get_busy_choice(&db, &id, now).unwrap().unwrap();
    let running = tokio::spawn({
        let (first, choice) = (Arc::clone(&first), choice.clone());
        async move { first.enqueue_busy_choice(&choice).await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), paused.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();

    assert!(confirmation_ready(&db, &busy_ready_marker(&id, 20, 99), now).unwrap());
    let saved = list_prompt_intakes(&db).unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].raw_prompt, "keep this exact request");
    assert!(list(&db).unwrap().is_empty());
    first.enqueue_busy_choice(&choice).await.unwrap();
    assert_eq!(paused.calls.load(Ordering::SeqCst), 1);
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    drop(first);

    let (_, restarted) = support::executor(
        &temp,
        db.clone(),
        bridge,
        Arc::clone(&backend),
        Arc::new(RecordingPreprocessor::new()),
    );
    assert_eq!(
        restarted.recover_prompt_intakes_on_startup().await.unwrap(),
        1
    );
    assert_eq!(restarted.recover_prompt_intakes().await.unwrap(), 0);
    restarted.enqueue_busy_choice(&choice).await.unwrap();
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].job_id, saved[0].job_id);
    assert_eq!(jobs[0].state, QueueJobState::Pending);
    assert_eq!(jobs[0].prompt, "prepared:managed:keep this exact request");
    assert!(list_prompt_intakes(&db).unwrap().is_empty());
    assert!(backend.starts.lock().await.is_empty());
    assert!(backend.forks.lock().await.is_empty());
}
