//! Authenticated proposal producer. No server request or queue execution is issued.
use super::{MessageContext, MessageWorkerError};
use crate::{
    completion_worker::IdempotentChunk, queue_runner::TurnBackend,
    restart_readiness::drain::AdmissionPermit,
};
use cdr_discord::components::abandonment_decision_rows;
use cdr_store::{StoreError, async_resolution::abandonment as store, delivery_receipt};
use sha2::{Digest, Sha256};
use std::time::Duration;
use twilight_model::channel::Message;

const DOMAIN: &str = "recovery-abandonment-proposal-v1";

pub(super) async fn propose<B: TurnBackend>(
    message: &Message,
    job: &str,
    permit: Option<AdmissionPermit>,
    context: &MessageContext<'_, B>,
) -> Result<(), MessageWorkerError> {
    let _permit = permit.ok_or_else(|| invalid("proposal requires live normal admission"))?;
    if message.author.bot {
        return Err(invalid("proposal requires the original human owner"));
    }
    let db = context.executor.mirror_db();
    let channel = sql_id(message.channel_id.get())?;
    let owner = sql_id(message.author.id.get())?;
    let target = store::command_target(db, job, channel, owner)?;
    let _lock = tokio::time::timeout(
        Duration::from_secs(2),
        context.executor.control_lock(&target),
    )
    .await
    .map_err(|_| invalid("proposal target is busy; no decision was recorded"))??;
    if store::command_target(db, job, channel, owner)? != target {
        return Err(invalid(
            "proposal target changed while waiting for the lock",
        ));
    }
    let ingress_id = format!("message:{}", message.id);
    let saved = cdr_store::ingress::get(db, &ingress_id)?
        .ok_or_else(|| invalid("proposal ingress is unavailable"))?;
    if saved.event_id != Some(sql_id(message.id.get())?)
        || saved.source_message_id != saved.event_id
        || saved.application_id.is_some()
        || saved.channel_id != channel
        || saved.owner_user_id != owner
        || saved.target_thread_id.as_deref() != Some(target.as_str())
        || saved.payload["content"] != message.content
        || saved.payload["processing_mode"] != "normal"
    {
        return Err(invalid(
            "proposal differs from its frozen authenticated ingress",
        ));
    }
    let now = super::custody::now()?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    let proposal = store::propose(
        db,
        &store::ProposalInput {
            proposal_id: &id,
            job_id: job,
            ingress_id: &ingress_id,
            application_id: sql_id(context.application_id.get())?,
            now,
            expires_at: now + 120.0,
        },
    )?;
    deliver(context, &proposal).await?;
    cdr_store::ingress::record_result(
        db,
        &ingress_id,
        &serde_json::json!({
            "kind":"abandonment_proposal","proposal_id":proposal.id,"revision":proposal.revision,
            "decision_recorded":false,"request_started":false,
        }),
        super::custody::now()?,
    )?;
    Ok(())
}

async fn deliver<B: TurnBackend>(
    context: &MessageContext<'_, B>,
    proposal: &store::Proposal,
) -> Result<(), MessageWorkerError> {
    let db = context.executor.mirror_db();
    let channel = twilight_model::id::Id::new(
        u64::try_from(proposal.channel_id).map_err(|_| invalid("invalid proposal channel"))?,
    );
    let components = abandonment_decision_rows(&proposal.id, proposal.revision)?;
    let chunk = IdempotentChunk {
        domain: DOMAIN,
        logical_key: format!("{}:{}", proposal.id, proposal.revision),
        chunk_index: 0,
        content: proposal.review_text.clone(),
    };
    tokio::time::timeout(
        Duration::from_secs(10),
        crate::completion_worker::send_recorded_message_with_components(
            db,
            &context.http,
            channel,
            &chunk,
            &components,
        ),
    )
    .await
    .map_err(|_| invalid("proposal delivery is unconfirmed; no automatic resend"))??;
    let key =
        serde_json::to_string(&(channel.get(), DOMAIN, &chunk.logical_key, chunk.chunk_index))
            .map_err(StoreError::from)?;
    let bytes = serde_json::to_vec(&(&chunk.content, &components)).map_err(StoreError::from)?;
    let hash = hex::encode(Sha256::digest(bytes));
    let delivery_receipt::ReceiptState::Delivered(message) =
        delivery_receipt::begin(db, &key, &hash)?
    else {
        return Err(invalid("proposal has no confirmed exact message receipt"));
    };
    let message = message
        .parse::<i64>()
        .map_err(|_| invalid("proposal receipt identity is invalid"))?;
    store::bind_delivery(
        db,
        &proposal.id,
        message,
        &proposal.review_sha256,
        super::custody::now()?,
    )?;
    Ok(())
}

fn sql_id(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value)
        .map_err(|_| StoreError::Integrity("Discord ID exceeds SQLite range".into()))
}
fn invalid(reason: &str) -> MessageWorkerError {
    StoreError::Integrity(format!("saved-request proposal held: {reason}")).into()
}
