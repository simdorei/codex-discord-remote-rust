use super::*;
use crate::{
    discord_dispatch::{
        BoxDiscordFuture, DispatchOutcome, InteractionDispatcher, InteractionTransport,
    },
    restart_readiness::drain::{AdmissionGate, DrainFenceKey},
    soak::native_fixture,
    test_support::{app_fixture, message_fixture::MessageFixture},
};
use cdr_app_server::requests::AppRequest;
use cdr_discord::{
    components::{PublicationDecision, parse_component_id, publication_decision_rows},
    gateway::ingress::InteractionIngressTag,
    interaction::RoutedWork,
    interaction_access::InteractionAccessPolicy,
};
use cdr_store::{
    async_resolution::{self, publication as p},
    ingress, queue,
};
use serde_json::{Value, json};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::{sync::mpsc, time::Instant};
use twilight_model::{
    http::interaction::InteractionResponse,
    id::{Id, marker::InteractionMarker},
};

mod boundaries;
mod http;

const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TARGET: &str = async_resolution::REVIEWED_INCIDENT_THREAD;

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
    proposal: p::Proposal,
}

impl Fixture {
    async fn new(address: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let client = Arc::new(
            Client::builder()
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
        // Positive logger control, before any publication component is handled.
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
        let runtime = MessageFixture::with_server(&temp, client, server);
        let db = runtime.executor.mirror_db();
        cdr_store::dead_generation::activate_runtime(db, runtime.server.instance_id()).unwrap();
        cdr_store::schema::open_initialized(db)
            .unwrap()
            .execute(
                "UPDATE mirror_threads SET codex_thread_id=? WHERE codex_thread_id='thread-b'",
                [TARGET],
            )
            .unwrap();
        queue::enqueue(
            db,
            queue::NewQueueJob {
                job_id: "publication-pending",
                target_thread_id: TARGET,
                channel_id: 42,
                owner_user_id: Some(3),
                discord_message_id: None,
                app_server_generation: 1,
                prompt: "fixture original input",
                queued: true,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        let now = super::now().unwrap();
        let proposal = p::propose(
            db,
            &p::ProposalInput {
                proposal_id: ID,
                job_id: "publication-pending",
                application_id: 1,
                review_text: "Fixture exact review. This records intent, not execution.",
                review_context: &json!({"fixture":true,"publisher_exclusion":"not_granted"}),
                now,
                expires_at: now + 120.0,
            },
        )
        .unwrap();
        p::bind_delivery(db, ID, 60, &proposal.review_sha256, now).unwrap();
        Self {
            temp,
            runtime,
            gate: AdmissionGate::new(),
            proposal,
        }
    }

    async fn dispatch(&self, value: Value) -> Result<InboundInteractionWork, String> {
        let interaction = serde_json::from_value(value).map_err(|e| format!("{e}"))?;
        let (sender, mut receiver) = mpsc::channel(4);
        let dispatcher = InteractionDispatcher::new(
            Arc::new(Transport::default()),
            InteractionAccessPolicy {
                allow_all_channels: true,
                ..Default::default()
            },
            false,
            sender,
            self.runtime.executor.mirror_db(),
        )
        .with_admission_gate(self.gate.clone());
        let result = dispatcher
            .dispatch(&interaction, Instant::now(), InteractionIngressTag::Normal)
            .await
            .map_err(|e| e.to_string())?;
        if result != DispatchOutcome::Queued {
            return Err(format!("{result:?}"));
        }
        receiver.try_recv().map_err(|e| e.to_string())
    }

    async fn click(&self, event: u64, decision: &str) -> InboundInteractionWork {
        let custom = format!("codex_pub:v1:{ID}:{}:{decision}", self.proposal.revision);
        let work = self.dispatch(raw_click(event, &custom)).await.unwrap();
        assert!(work.admission_permit.is_some());
        let record = ingress::get(self.runtime.executor.mirror_db(), &work.custody_ingress_id)
            .unwrap()
            .unwrap();
        assert_eq!(record.target_thread_id.as_deref(), Some(TARGET));
        assert_eq!(
            record.payload["work"],
            serde_json::to_value(&work.work).unwrap()
        );
        assert!(
            ingress::begin_execution(
                self.runtime.executor.mirror_db(),
                &work.custody_ingress_id,
                "processing",
                None,
                super::now().unwrap()
            )
            .unwrap()
        );
        work
    }

    fn assert_held(&self, decisions: i64) {
        let db = self.runtime.executor.mirror_db();
        let rows = queue::list(db).unwrap();
        let pending = rows
            .iter()
            .find(|job| job.job_id == "publication-pending")
            .unwrap();
        assert_eq!(pending.state, queue::QueueJobState::Pending);
        assert_eq!(pending.attempt_count, 0);
        assert!(pending.turn_id.is_none());
        assert!(async_resolution::admission_held(db, TARGET).unwrap());
        let count: i64 = cdr_store::schema::open_initialized(db)
            .unwrap()
            .query_row(
                "SELECT count(*) FROM cdr_recovery_publication_decisions",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, decisions);
    }

    async fn finish(self) {
        self.runtime.server.close().await.unwrap();
        let rpc = app_fixture::rpc_log(&self.temp.path().join("rpc.jsonl"));
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
}

fn raw_click(event: u64, custom: &str) -> Value {
    let author =
        json!({"id":"3","username":"fixture","discriminator":"0001","avatar":null,"bot":false});
    json!({
        "id":event.to_string(),"application_id":"1","type":3,"token":"fixture",
        "version":1,"channel_id":"42","locale":"en-US","user":author,
        "data":{"component_type":2,"custom_id":custom},
        "message":{"id":"60","channel_id":"42","author":author,"content":"fixture review",
            "timestamp":"2020-02-02T02:02:02.020000+00:00","edited_timestamp":null,
            "tts":false,"mention_everyone":false,"mentions":[],"mention_roles":[],
            "attachments":[],"embeds":[],"pinned":false,"type":0},
        "entitlements":[],"authorizing_integration_owners":{}
    })
}

fn component(work: &InboundInteractionWork) -> &ComponentId {
    let RoutedWork::Component(component) = &work.work else {
        panic!("component required")
    };
    component
}

#[test]
fn publication_rows_are_exact_and_cannot_build_native_approval_responses() {
    let rows = serde_json::to_value(publication_decision_rows(ID, 1).unwrap()).unwrap();
    for (index, choice) in [(0, "a"), (1, "h")] {
        let custom = rows[0]["components"][index]["custom_id"].as_str().unwrap();
        assert_eq!(custom, format!("codex_pub:v1:{ID}:1:{choice}"));
        let parsed = parse_component_id(custom).unwrap();
        assert!(build_component_response(&parsed, &[], 1).is_err());
        assert!(standard_confirmation_plan(&parsed, "not-a-native-claim").is_err());
    }
    assert!(publication_decision_rows("bad", 1).is_err());
    assert!(publication_decision_rows(ID, 0).is_err());
}

#[tokio::test]
async fn actual_dispatch_exact_intent_and_duplicate_confirmation_never_start_work() {
    let http = http::start(false).await;
    let f = Fixture::new(&http.address).await;
    let work = f.click(501, "a").await;
    let first = handle_component_work(
        &work,
        component(&work),
        &f.runtime.executor,
        &f.runtime.server,
    )
    .await
    .unwrap();
    assert_eq!(
        first.plan.domain,
        "recovery-publication-intent-confirmation-v1"
    );
    let expected = first.plan.clone();
    first.deliver(f.runtime.http.clone()).await.unwrap();
    ingress::record_result(
        f.runtime.executor.mirror_db(),
        &work.custody_ingress_id,
        &json!({"action_completed":true}),
        super::now().unwrap(),
    )
    .unwrap();
    let duplicate = f.click(502, "a").await;
    let second = handle_component_work(
        &duplicate,
        component(&duplicate),
        &f.runtime.executor,
        &f.runtime.server,
    )
    .await
    .unwrap();
    assert_eq!(second.plan, expected);
    second.deliver(f.runtime.http.clone()).await.unwrap();
    f.assert_held(1);
    drop(work);
    drop(duplicate);
    f.finish().await;
    let traffic = http.finish().await;
    assert_eq!(
        traffic
            .iter()
            .filter(|(request, _)| request.starts_with("POST "))
            .count(),
        1
    );
    assert_eq!(
        traffic
            .iter()
            .filter(|(request, _)| request.starts_with("PATCH "))
            .count(),
        2
    );
}

#[tokio::test]
async fn unknown_confirmation_response_never_reposts_or_reexecutes_on_another_click() {
    let http = http::start(true).await;
    let f = Fixture::new(&http.address).await;
    for event in [501, 502] {
        let work = f.click(event, "a").await;
        let confirmation = handle_component_work(
            &work,
            component(&work),
            &f.runtime.executor,
            &f.runtime.server,
        )
        .await
        .unwrap();
        assert!(confirmation.deliver(f.runtime.http.clone()).await.is_err());
        f.assert_held(1);
    }
    f.finish().await;
    let traffic = http.finish().await;
    assert_eq!(traffic.len(), 1);
    assert!(traffic[0].0.starts_with("POST "));
}

#[test]
fn publication_error_identity_cannot_alias_another_decision_revision_or_native_claim() {
    use crate::discord_dispatch::delivery_identity::{
        component_claim_identity, component_delivery_key,
    };
    let identity = |revision, decision| ComponentId::RecoveryPublicationDecision {
        proposal_id: ID.into(),
        revision,
        decision,
    };
    let first = identity(1, PublicationDecision::ApproveExact);
    let key = |component: &ComponentId| {
        component_delivery_key(Id::new(501), Some(Id::new(60)), component, None)
    };
    assert_eq!(component_claim_identity(Some(Id::new(60)), &first), None);
    assert_ne!(
        key(&first),
        key(&identity(1, PublicationDecision::KeepHeld))
    );
    assert_ne!(
        key(&first),
        key(&identity(2, PublicationDecision::ApproveExact))
    );
    assert_ne!(
        key(&first),
        key(&ComponentId::AsyncChoice {
            question_id: ID.repeat(2),
            option: 0,
        })
    );
}
