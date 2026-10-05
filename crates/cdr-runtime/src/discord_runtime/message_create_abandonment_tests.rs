//! Actual gateway/dispatcher fixtures, with no operating Discord or app-server.
use super::{dispatch_message_create, prepare_message_create};
use crate::{
    component_worker::{
        build_component_response, handle_component_work, standard_confirmation_plan,
    },
    discord_dispatch::{
        BoxDiscordFuture, DispatchOutcome, InboundInteractionWork, InteractionDispatcher,
        InteractionProcessingMode, InteractionTransport,
    },
    message_worker::{ErrorReportTarget, process_admitted_gateway_message},
    restart_readiness::drain::{AdmissionGate, DrainFenceKey},
    soak::native_fixture,
    test_support::{app_fixture, message_fixture::MessageFixture},
};
use cdr_app_server::{ResidentAppServer, requests::AppRequest};
use cdr_discord::{
    components::{ComponentId, parse_component_id},
    gateway::ingress::InteractionIngressTag,
    interaction::RoutedWork,
    interaction_access::InteractionAccessPolicy,
};
use cdr_store::{
    async_resolution::{self, abandonment as store},
    ingress, queue,
};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{sync::mpsc, time::Instant};
use twilight_model::{
    channel::Message,
    http::interaction::InteractionResponse,
    id::{Id, marker::InteractionMarker},
};

#[path = "message_create_abandonment_tests/cases.rs"]
mod cases;
#[path = "message_create_abandonment_tests/http.rs"]
mod http;

const JOB: &str = "b3d5a1a3-5c3e-4764-967b-0cef767efde9";
const SIBLING: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const TARGET: &str = async_resolution::REVIEWED_INCIDENT_THREAD;

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

#[derive(Default)]
struct Transport {
    acknowledgements: AtomicUsize,
}

impl InteractionTransport for Transport {
    fn acknowledge<'a>(
        &'a self,
        _: Id<InteractionMarker>,
        _: &'a str,
        _: &'a InteractionResponse,
    ) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async move {
            self.acknowledgements.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
    fn update<'a>(&'a self, _: &'a str, _: &'a str) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

struct Fixture {
    temp: tempfile::TempDir,
    runtime: MessageFixture,
    gate: AdmissionGate,
    transport: Arc<Transport>,
}

impl Fixture {
    async fn new(address: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let http = Arc::new(
            twilight_http::Client::builder()
                .token("fixture".into())
                .proxy(address.into(), true)
                .ratelimiter(None)
                .build(),
        );
        let mut config = native_fixture::config("async-question");
        config.environment.insert(
            "CDR_ACTION_RPC_LOG".into(),
            temp.path().join("rpc.jsonl").to_string_lossy().into_owned(),
        );
        let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
        server
            .execute(
                AppRequest {
                    method: "test/active-turn",
                    params: json!({}),
                    timeout: Duration::from_secs(2),
                },
                None,
            )
            .await
            .unwrap();
        let runtime = MessageFixture::with_server(&temp, http, server);
        let db = runtime.executor.mirror_db();
        cdr_store::dead_generation::activate_runtime(db, runtime.server.instance_id()).unwrap();
        let connection = cdr_store::schema::open_initialized(db).unwrap();
        connection
            .execute(
                "INSERT INTO codex_mutation_runtime VALUES(1,'abandonment-fixture')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE mirror_threads SET codex_thread_id=? WHERE codex_thread_id='thread-b'",
                [TARGET],
            )
            .unwrap();
        for (job, event) in [(JOB, 70), (SIBLING, 71)] {
            queue::enqueue(
                db,
                queue::NewQueueJob {
                    job_id: job,
                    target_thread_id: TARGET,
                    channel_id: 42,
                    owner_user_id: Some(3),
                    discord_message_id: Some(event),
                    app_server_generation: 1,
                    prompt: "private original input",
                    queued: true,
                    ack_sent: true,
                    created_at: 1.0,
                },
            )
            .unwrap();
        }
        Self {
            temp,
            runtime,
            gate: AdmissionGate::new(),
            transport: Arc::new(Transport::default()),
        }
    }

    fn db(&self) -> &std::path::Path {
        self.runtime.executor.mirror_db()
    }

    fn sql(&self, statement: &str) {
        cdr_store::schema::open_initialized(self.db())
            .unwrap()
            .execute_batch(statement)
            .unwrap();
    }

    fn count(&self, table: &str) -> i64 {
        cdr_store::schema::open_initialized(self.db())
            .unwrap()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    fn proposal(&self) -> store::Proposal {
        let id: String = cdr_store::schema::open_initialized(self.db())
            .unwrap()
            .query_row(
                "SELECT id FROM cdr_recovery_abandonment_proposals ORDER BY revision DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let revision: i64 = cdr_store::schema::open_initialized(self.db())
            .unwrap()
            .query_row(
                "SELECT revision FROM cdr_recovery_abandonment_proposals WHERE id=?",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        store::delivered_proposal(self.db(), &id, revision)
            .unwrap()
            .proposal
    }

    async fn message(&self, input: Message) -> Result<(), String> {
        let context = self.runtime.context(self.temp.path());
        let resolver = self.runtime.executor.settings_resolver();
        let errors = Mutex::new(Vec::new());
        dispatch_message_create(
            ErrorReportTarget::from_message(&input),
            || {
                prepare_message_create(
                    input.clone(),
                    None,
                    context.config,
                    self.db(),
                    &self.gate,
                    false,
                    &resolver,
                )
            },
            |admitted| process_admitted_gateway_message(admitted, &context),
            |_, error| {
                errors.lock().unwrap().push(error.to_string());
                std::future::ready(())
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        match errors.into_inner().unwrap().into_iter().next() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    async fn propose(&self, event: u64) {
        self.message(raw_message(event, &format!("!discard-request {JOB}")))
            .await
            .unwrap();
        assert_eq!(self.count("cdr_recovery_abandonment_deliveries"), 1);
    }

    async fn dispatch(&self, value: Value) -> Result<InboundInteractionWork, String> {
        let interaction = serde_json::from_value(value).map_err(|e| format!("{e}"))?;
        let (sender, mut receiver) = mpsc::channel(4);
        let dispatcher = InteractionDispatcher::new(
            self.transport.clone(),
            InteractionAccessPolicy {
                allow_all_channels: true,
                ..Default::default()
            },
            false,
            sender,
            self.db(),
        )
        .with_admission_gate(self.gate.clone());
        let outcome = dispatcher
            .dispatch(&interaction, Instant::now(), InteractionIngressTag::Normal)
            .await
            .map_err(|e| e.to_string())?;
        if outcome != DispatchOutcome::Queued {
            return Err(format!("{outcome:?}"));
        }
        receiver.try_recv().map_err(|e| e.to_string())
    }

    async fn click(&self, event: u64, choice: &str) -> InboundInteractionWork {
        let p = self.proposal();
        self.dispatch(raw_click(
            event,
            &format!("codex_discard:v1:{}:{}:{choice}", p.id, p.revision),
        ))
        .await
        .unwrap()
    }

    fn begin(&self, work: &InboundInteractionWork) {
        assert!(
            ingress::begin_execution(
                self.db(),
                &work.custody_ingress_id,
                "processing",
                None,
                now()
            )
            .unwrap()
        );
    }

    fn held(&self, decisions: i64) {
        let rows = queue::list(self.db()).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|j| j.state == queue::QueueJobState::Pending && j.turn_id.is_none())
        );
        assert!(rows.iter().any(|j| j.job_id == JOB && j.attempt_count == 0));
        assert_eq!(self.count("cdr_recovery_abandonment_decisions"), decisions);
        assert!(async_resolution::admission_held(self.db(), TARGET).unwrap());
    }

    async fn finish(self) {
        self.runtime.server.close().await.unwrap();
        assert_no_rpc(&self.temp);
    }
}

fn assert_no_rpc(temp: &tempfile::TempDir) {
    let rpc = app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
    assert!(rpc.iter().any(|v| v["method"] == "test/active-turn"));
    assert!(!rpc.iter().any(|v| matches!(
        v["method"].as_str(),
        Some(
            "turn/start"
                | "turn/steer"
                | "thread/resume"
                | "thread/fork"
                | "thread/settings/update"
        )
    )));
}

fn raw_message(event: u64, content: &str) -> Message {
    serde_json::from_value(json!({
        "attachments":[],"author":{"id":"3","avatar":null,"bot":false,"discriminator":"0001","username":"fixture"},
        "channel_id":"42","content":content,"edited_timestamp":null,"embeds":[],"id":event.to_string(),
        "mention_everyone":false,"mention_roles":[],"mentions":[],"pinned":false,
        "timestamp":"2020-02-02T02:02:02.020000+00:00","tts":false,"type":0
    })).unwrap()
}

fn raw_click(event: u64, custom: &str) -> Value {
    let author =
        json!({"id":"3","username":"fixture","discriminator":"0001","avatar":null,"bot":false});
    json!({
        "id":event.to_string(),"application_id":"1","type":3,"token":"fixture","version":1,
        "channel_id":"42","locale":"en-US","user":author,
        "data":{"component_type":2,"custom_id":custom},
        "message":{"id":"60","channel_id":"42","author":author,"content":"fixture review",
            "timestamp":"2020-02-02T02:02:02.020000+00:00","edited_timestamp":null,"tts":false,
            "mention_everyone":false,"mentions":[],"mention_roles":[],"attachments":[],
            "embeds":[],"pinned":false,"type":0},
        "entitlements":[],"authorizing_integration_owners":{}
    })
}

fn component(work: &InboundInteractionWork) -> &ComponentId {
    let RoutedWork::Component(value) = &work.work else {
        panic!("component required")
    };
    value
}
