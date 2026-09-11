use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use cdr_app_server::ResidentAppServer;
use cdr_discord::components::ComponentId;
use cdr_discord::interaction::RoutedWork;
use cdr_store::ingress::{IngressKind, NewIngress};
use serde_json::{Value, json};
use twilight_model::id::{
    Id,
    marker::{ApplicationMarker, ChannelMarker, InteractionMarker, MessageMarker, UserMarker},
};

use crate::action_executor::ActionExecutor;
use crate::bridge_state::BridgeState;
use crate::discord_dispatch::{InboundInteractionWork, InteractionProcessingMode};
use crate::queue_runner::{BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord};

#[derive(Default)]
pub(super) struct CountingBackend {
    pub calls: AtomicUsize,
}

impl TurnBackend for CountingBackend {
    fn generation(&self) -> u64 {
        1
    }

    fn active_turn_id<'a>(&'a self, _: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(None) })
    }

    fn read_turns<'a>(&'a self, _: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(Vec::new()) })
    }

    fn resume_thread<'a>(&'a self, _: &'a str) -> BoxBackendFuture<'a, ()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }

    fn start_turn<'a>(&'a self, _: &'a str, _: &'a str) -> BoxBackendFuture<'a, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok("unexpected-turn".into()) })
    }
}

pub(super) fn executor(
    temp: &tempfile::TempDir,
    database: PathBuf,
    backend: Arc<CountingBackend>,
) -> ActionExecutor<CountingBackend> {
    ActionExecutor::new(
        temp.path().join("state.sqlite"),
        database.clone(),
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        Arc::new(QueueCoordinator::new(database, backend)),
    )
}

pub(super) fn stage(database: &Path, ingress_id: &str, event_id: i64) {
    cdr_store::ingress::admit(database, &new_ingress(ingress_id, event_id, json!({}))).unwrap();
    assert!(cdr_store::ingress::acknowledge(database, ingress_id, 1.0).unwrap());
}

pub(super) fn admit_busy(
    database: &Path,
    ingress_id: &str,
    event_id: i64,
    choice_id: &str,
) -> cdr_store::ingress::IngressAdmission {
    cdr_store::ingress::admit_busy_interaction(
        database,
        &new_ingress(ingress_id, event_id, json!({"work":"busy"})),
        choice_id,
        "queue",
    )
    .unwrap()
}

fn new_ingress(ingress_id: &str, event_id: i64, payload: Value) -> NewIngress {
    NewIngress {
        ingress_id: ingress_id.into(),
        kind: IngressKind::Interaction,
        event_id: Some(event_id),
        application_id: Some(2),
        channel_id: 10,
        owner_user_id: 20,
        source_message_id: Some(91),
        payload,
        target_thread_id: None,
        canonical_owner: Some(ingress_id.into()),
        now: f64::from(i32::try_from(event_id).unwrap()),
    }
}

pub(super) fn work(
    database: &Path,
    ingress_id: &str,
    interaction_id: u64,
    component: ComponentId,
    processing_mode: InteractionProcessingMode,
    authorized_busy_choice: Option<cdr_store::claims::BusyChoice>,
) -> InboundInteractionWork {
    InboundInteractionWork {
        application_id: Id::<ApplicationMarker>::new(2),
        interaction_id: Id::<InteractionMarker>::new(interaction_id),
        channel_id: Id::<ChannelMarker>::new(10),
        user_id: Id::<UserMarker>::new(20),
        source_message_id: Some(Id::<MessageMarker>::new(91)),
        interaction_token: "not-used".into(),
        work: RoutedWork::Component(component),
        processing_mode,
        custody_database: database.into(),
        custody_ingress_id: ingress_id.into(),
        authorized_busy_choice,
        admission_permit: None,
    }
}

pub(super) fn reject_confirmation_ready_writes(database: &Path) {
    rusqlite::Connection::open(database)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_ready BEFORE INSERT ON persistent_component_claims
             WHEN NEW.claim_key LIKE 'confirmation-ready:%'
             BEGIN SELECT RAISE(ABORT, 'fixture rejected confirmation-ready marker'); END;",
        )
        .unwrap();
}

pub(super) fn approval_response_count(log: &Path) -> usize {
    std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry["id"] == "server-approval" && entry.get("result").is_some())
        .count()
}

pub(super) async fn owned_approval(database: &Path, server: &ResidentAppServer) -> ComponentId {
    let generation = i64::try_from(server.generation()).unwrap();
    cdr_store::queue::enqueue(
        database,
        cdr_store::queue::NewQueueJob {
            job_id: "approval-owner",
            target_thread_id: "thread-a",
            channel_id: 10,
            owner_user_id: Some(20),
            discord_message_id: Some(90),
            app_server_generation: generation,
            prompt: "fixture approval",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    cdr_store::queue::begin_attempt(database, "approval-owner", &[], generation).unwrap();
    cdr_store::queue::mark_running(database, "approval-owner", "turn-a", generation).unwrap();
    server
        .request(
            "test/requestApproval",
            json!({}),
            std::time::Duration::from_secs(1),
            Some(server.generation()),
        )
        .await
        .unwrap();
    let pending = server.pending_server_requests(None).await.unwrap();
    let prompt =
        crate::server_prompt::build_server_prompt(&pending[0], server.generation()).unwrap();
    let row = serde_json::to_value(&prompt.components[0]).unwrap();
    cdr_discord::components::parse_component_id(row["components"][0]["custom_id"].as_str().unwrap())
        .unwrap()
}

pub(super) async fn start_fake_server(
    temp: &tempfile::TempDir,
) -> (Arc<ResidentAppServer>, PathBuf) {
    let log = temp
        .path()
        .join(format!("app-server-{}.jsonl", uuid::Uuid::new_v4()));
    let mut config = crate::test_support::native_fixture::config("interaction");
    config.environment.insert(
        "CDR_INTERACTION_RPC_LOG".into(),
        log.to_string_lossy().into(),
    );
    (
        Arc::new(ResidentAppServer::start(config).await.unwrap()),
        log,
    )
}
