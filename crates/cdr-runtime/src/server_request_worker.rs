use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub mod delivery_identity;
mod error;
pub mod recovery;
pub mod sent_cache;

use cdr_app_server::{ResidentAppServer, ResidentServerRequestEvent, ServerRequest};
use cdr_discord::delivery::{DeliveryPolicy, deliver_text_indexed};
use cdr_discord::text::split_delivery_chunks;
use tokio::sync::{broadcast, watch};
use tokio::time::{Instant, sleep_until};
use twilight_http::Client;
use twilight_model::id::{Id, marker::ChannelMarker};

use crate::server_prompt::{ServerPrompt, build_server_prompt};
use delivery_identity::{PromptDeliveryIdentity, PromptIdentityMaterial};
use error::{ServerPromptSendError, ServerRequestWorkerError};
use recovery::{
    RetrySchedule, SnapshotError, attempt_all, cancel_on_shutdown, consistent_snapshot,
    validate_generation,
};
use sent_cache::SentRequestCache;

#[cfg(test)]
mod receipt_contract;

enum WorkerAction {
    Recover,
    Process {
        generation: u64,
        request: ServerRequest,
    },
}

impl WorkerAction {
    const fn is_recovery(&self) -> bool {
        matches!(self, Self::Recover)
    }
}

pub async fn run_server_request_worker(
    mut receiver: broadcast::Receiver<ResidentServerRequestEvent>,
    server: Arc<ResidentAppServer>,
    mirror_db: PathBuf,
    _default_channel_id: Option<u64>,
    http: Arc<Client>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut worker = ServerRequestWorker {
        server,
        mirror_db,
        http,
        sent: SentRequestCache::default(),
    };
    let mut retries = RetrySchedule::default();
    let mut initial = true;
    loop {
        let action = if initial {
            initial = false;
            WorkerAction::Recover
        } else {
            let retry_at = retries
                .deadline()
                .unwrap_or_else(|| Instant::now() + Duration::from_hours(24));
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { return; }
                    continue;
                }
                () = sleep_until(retry_at), if retries.deadline().is_some() => {
                    retries.begin_retry();
                    WorkerAction::Recover
                }
                event = receiver.recv() => match event {
                    Ok(ResidentServerRequestEvent::Request { generation, request }) => {
                        WorkerAction::Process { generation, request }
                    }
                    Ok(ResidentServerRequestEvent::Gap { generation, skipped }) => {
                        worker.server.mark_idle_observation_gap();
                        eprintln!("app_server_request_gap generation={generation} skipped={skipped}");
                        WorkerAction::Recover
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        worker.server.mark_idle_observation_gap();
                        eprintln!("server_request_worker_gap skipped={skipped}");
                        WorkerAction::Recover
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        };
        let recovered = action.is_recovery();
        let Some(result) = cancel_on_shutdown(&mut shutdown, worker.perform(action)).await else {
            return;
        };
        match result {
            Ok(()) if recovered => retries.clear(),
            Ok(()) => {}
            Err(error) => {
                ServerRequestWorker::report(Err(error));
                retries.schedule_failure(Instant::now());
            }
        }
    }
}

struct ServerRequestWorker {
    server: Arc<ResidentAppServer>,
    mirror_db: PathBuf,
    http: Arc<Client>,
    sent: SentRequestCache,
}

impl ServerRequestWorker {
    async fn perform(&mut self, action: WorkerAction) -> Result<(), ServerRequestWorkerError> {
        match action {
            WorkerAction::Recover => self.recover().await,
            WorkerAction::Process {
                generation,
                request,
            } => self.process(generation, request).await,
        }
    }

    async fn recover(&mut self) -> Result<(), ServerRequestWorkerError> {
        let server = self.server.as_ref();
        let (generation, requests) = consistent_snapshot(
            3,
            || server.generation(),
            || server.pending_server_requests(None),
        )
        .await
        .map_err(|error| match error {
            SnapshotError::Fetch(error) => ServerRequestWorkerError::AppServer(error),
            SnapshotError::Unstable { attempts } => {
                ServerRequestWorkerError::UnstableSnapshot { attempts }
            }
        })?;
        attempt_all(self, requests, |worker, request| {
            Box::pin(worker.process(generation, request))
        })
        .await
    }

    async fn process(
        &mut self,
        generation: u64,
        request: ServerRequest,
    ) -> Result<(), ServerRequestWorkerError> {
        validate_generation(generation, self.server.generation())?;
        let authority = crate::server_prompt_authority::verify(
            &self.mirror_db,
            &self.server,
            &request,
            generation,
        )
        .await?;
        let prompt = build_server_prompt(&request, generation)?;
        let material = PromptIdentityMaterial {
            generation,
            occurrence: &request.occurrence,
            request_id: &request.id,
            method: &request.method,
            thread_id: &prompt.thread_id,
            text: &prompt.text,
            components: &prompt.components,
        };
        let identity =
            PromptDeliveryIdentity::new(&material).map_err(ServerRequestWorkerError::RequestId)?;
        let key = identity.logical_key().to_owned();
        if self.sent.contains(generation, &key) {
            return Ok(());
        }
        let channel_id = Id::new(authority.channel_id);
        self.send_prompt(channel_id, &identity, &prompt, &request)
            .await?;
        self.sent.remember(generation, key);
        Ok(())
    }

    async fn send_prompt(
        &self,
        channel_id: Id<ChannelMarker>,
        identity: &PromptDeliveryIdentity,
        prompt: &ServerPrompt,
        request: &ServerRequest,
    ) -> Result<(), ServerRequestWorkerError> {
        let policy = DeliveryPolicy {
            retry_delays: Vec::new(),
            chunk_markers: true,
        };
        let total = split_delivery_chunks(&prompt.text, policy.chunk_markers).len();
        deliver_text_indexed(&prompt.text, &policy, |part, chunk| {
            let http = Arc::clone(&self.http);
            let server = Arc::clone(&self.server);
            let delivery = identity.chunk(part, total);
            let generation = identity.generation();
            let components = if delivery.attach_components {
                prompt.components.clone()
            } else {
                Vec::new()
            };
            async move {
                validate_generation(generation, server.generation())
                    .map_err(ServerPromptSendError::from)?;
                let authority = crate::server_prompt_authority::verify(
                    &self.mirror_db,
                    &server,
                    request,
                    generation,
                )
                .await?;
                if authority.channel_id != channel_id.get() {
                    return Err(
                        crate::server_prompt_authority::PromptAuthorityError::Invalid(
                            "original delivery channel changed",
                        )
                        .into(),
                    );
                }
                crate::completion_worker::send_recorded_message_with_components(
                    &self.mirror_db,
                    &http,
                    channel_id,
                    &crate::completion_worker::IdempotentChunk {
                        domain: delivery.domain,
                        logical_key: delivery.logical_key,
                        chunk_index: delivery.chunk_index,
                        content: chunk,
                    },
                    &components,
                )
                .await
                .map_err(ServerPromptSendError::from)
            }
        })
        .await
        .map_err(ServerRequestWorkerError::Delivery)?;
        Ok(())
    }

    fn report(result: Result<(), ServerRequestWorkerError>) {
        if let Err(error) = result {
            eprintln!("server_request_worker_error: {error}");
        }
    }
}
