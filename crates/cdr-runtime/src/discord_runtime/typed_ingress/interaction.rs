use std::future::{Future, pending};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use cdr_discord::gateway::ingress::{InteractionIngress, InteractionIngressTag};
use cdr_discord::http::DiscordHttp;
use cdr_discord::interaction_access::InteractionAccessPolicy;
use futures_util::{StreamExt, stream::FuturesUnordered};
use tokio::{
    sync::{mpsc, watch},
    time::{Duration, Instant, sleep_until},
};
use twilight_http::Client;

use super::super::DiscordRuntimeError;
use super::TypedIngressContext;
use crate::discord_dispatch::{
    AutocompleteCatalog, InboundInteractionWork, InteractionClaimCache, InteractionDispatcher,
};
use crate::restart_readiness::drain::AdmissionGate;

const NORMAL_CONCURRENCY: usize = 16;
const RESERVED_CONCURRENCY: usize = 4;
const INTERACTION_CLAIM_CAPACITY: usize = 4_096;
const RESERVED_SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_secs(13);

#[path = "interaction_failure.rs"]
mod failure;

type InteractionFuture =
    Pin<Box<dyn Future<Output = Result<(), DiscordRuntimeError>> + Send + 'static>>;

#[derive(Clone)]
pub(in crate::discord_runtime) struct InteractionResources {
    policy: Arc<InteractionAccessPolicy>,
    work: mpsc::Sender<InboundInteractionWork>,
    autocomplete: Arc<AutocompleteCatalog>,
    claims: InteractionClaimCache,
    admission: AdmissionGate,
    ingress_db: PathBuf,
}

impl InteractionResources {
    pub(in crate::discord_runtime) fn new(
        policy: Arc<InteractionAccessPolicy>,
        work: mpsc::Sender<InboundInteractionWork>,
        autocomplete: Arc<AutocompleteCatalog>,
        admission: AdmissionGate,
        ingress_db: PathBuf,
    ) -> Self {
        Self {
            policy,
            work,
            autocomplete,
            claims: InteractionClaimCache::new(INTERACTION_CLAIM_CAPACITY),
            admission,
            ingress_db,
        }
    }
}

#[derive(Clone)]
pub(super) struct DiscordInteractionHandler {
    http: Arc<Client>,
    qa_enabled: bool,
    resources: InteractionResources,
    settings_resolver: crate::settings_binding::SettingsTargetResolver,
}

impl DiscordInteractionHandler {
    pub(super) fn from_context(context: &TypedIngressContext) -> Self {
        Self {
            http: Arc::clone(&context.http),
            qa_enabled: context.config.qa_commands,
            resources: context.interaction.clone(),
            settings_resolver: context.executor.settings_resolver(),
        }
    }
}

trait InteractionHandler: Clone + Send + Sync + 'static {
    fn handle(&self, item: InteractionIngress) -> InteractionFuture;
}

impl InteractionHandler for DiscordInteractionHandler {
    fn handle(&self, item: InteractionIngress) -> InteractionFuture {
        let handler = self.clone();
        Box::pin(async move {
            let api = Arc::new(DiscordHttp::new(
                Arc::clone(&handler.http),
                item.event.application_id,
            ));
            let mut policy = handler.resources.policy.as_ref().clone();
            crate::discord_runtime::bootstrap::refresh_mirror_policy(
                &mut policy,
                &handler.resources.ingress_db,
            )?;
            let dispatcher = InteractionDispatcher::new(
                api,
                Arc::new(policy),
                handler.qa_enabled,
                handler.resources.work.clone(),
                handler.resources.ingress_db.clone(),
            )
            .with_autocomplete_catalog(Arc::clone(&handler.resources.autocomplete))
            .with_claim_cache(handler.resources.claims.clone())
            .with_admission_gate(handler.resources.admission.clone());
            let dispatcher = dispatcher.with_settings_resolver(handler.settings_resolver.clone());
            let _ = dispatcher
                .dispatch(&item.event, item.received_at, item.tag)
                .await?;
            Ok(())
        })
    }
}

#[derive(Clone, Copy)]
enum InteractionLane {
    Normal,
    Reserved,
}

impl InteractionLane {
    const fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal-interaction",
            Self::Reserved => "reserved-interaction",
        }
    }

    const fn accepts(self, tag: InteractionIngressTag) -> bool {
        matches!(
            (self, tag),
            (Self::Normal, InteractionIngressTag::Normal)
                | (
                    Self::Reserved,
                    InteractionIngressTag::Busy | InteractionIngressTag::Stopping
                )
        )
    }

    fn shutdown_deadline(self) -> Option<Instant> {
        match self {
            Self::Normal => None,
            Self::Reserved => Some(Instant::now() + RESERVED_SHUTDOWN_DRAIN_TIMEOUT),
        }
    }
}

pub(super) async fn run_normal(
    receiver: mpsc::Receiver<InteractionIngress>,
    handler: DiscordInteractionHandler,
    shutdown: watch::Receiver<bool>,
) -> Result<(), DiscordRuntimeError> {
    run_lane(
        receiver,
        InteractionLane::Normal,
        NORMAL_CONCURRENCY,
        handler,
        shutdown,
    )
    .await
}

pub(super) async fn run_reserved(
    receiver: mpsc::Receiver<InteractionIngress>,
    handler: DiscordInteractionHandler,
    shutdown: watch::Receiver<bool>,
) -> Result<(), DiscordRuntimeError> {
    run_lane(
        receiver,
        InteractionLane::Reserved,
        RESERVED_CONCURRENCY,
        handler,
        shutdown,
    )
    .await
}

async fn run_lane<H: InteractionHandler>(
    mut receiver: mpsc::Receiver<InteractionIngress>,
    lane: InteractionLane,
    concurrency: usize,
    handler: H,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), DiscordRuntimeError> {
    let mut active = FuturesUnordered::new();
    let mut drain_deadline = None;
    let mut input_closed = false;
    loop {
        if drain_deadline.is_none() && *shutdown.borrow() {
            let Some(deadline) = lane.shutdown_deadline() else {
                return Ok(());
            };
            drain_deadline = Some(deadline);
        }
        if input_closed && active.is_empty() {
            return Ok(());
        }
        tokio::select! {
            biased;
            () = async {
                match drain_deadline {
                    Some(deadline) => sleep_until(deadline).await,
                    None => pending().await,
                }
            } => {
                return Err(DiscordRuntimeError::TypedIngressDrainTimeout(lane.name()));
            }
            changed = shutdown.changed(), if drain_deadline.is_none() => {
                if changed.is_err() || *shutdown.borrow() {
                    let Some(deadline) = lane.shutdown_deadline() else {
                        return Ok(());
                    };
                    drain_deadline = Some(deadline);
                }
            }
            item = receiver.recv(), if !input_closed && active.len() < concurrency => {
                let Some(item) = item else {
                    if drain_deadline.is_none() {
                        return Err(DiscordRuntimeError::TypedIngressClosed(lane.name()));
                    }
                    input_closed = true;
                    continue;
                };
                if !lane.accepts(item.tag) {
                    return Err(DiscordRuntimeError::TypedInteractionTag {
                        lane: lane.name(),
                        tag: item.tag,
                    });
                }
                let interaction_id = item.event.id.get();
                let token = item.event.token.clone();
                let processing = handler.handle(item);
                active.push(async move { (interaction_id, token, processing.await) });
            }
            result = active.next(), if !active.is_empty() => {
                let (id, token, result) = result.expect("non-empty interaction future set");
                failure::report_event_result(lane.name(), id, &token, result)?;
            }
        }
    }
}

#[cfg(test)]
#[path = "interaction_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "interaction_failure_tests.rs"]
mod failure_tests;

#[cfg(test)]
#[path = "settings_rejection_tests.rs"]
mod settings_rejection_tests;
