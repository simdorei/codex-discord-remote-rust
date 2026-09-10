//! A /new normal initial-response PATCH has its own durable, single-attempt receipt.
use super::InteractionWorkerError;
use crate::{action_executor::ActionResult, discord_dispatch::InboundInteractionWork};
use cdr_discord::{delivery::DeliveryFailure, http::DiscordHttp};
use cdr_store::{
    StoreError,
    delivery_receipt::{self, ReceiptState},
};
use sha2::{Digest, Sha256};
use std::path::Path;

pub(super) async fn deliver(
    work: &InboundInteractionWork,
    result: &ActionResult,
    db: &Path,
    api: &DiscordHttp,
) -> Result<bool, InteractionWorkerError> {
    let Some(record) = cdr_store::new_reply::get_by_ingress(db, &work.custody_ingress_id)? else {
        return Ok(false);
    };
    if result.text != record.identity.acknowledgement || result.ui.is_some() {
        return Err(StoreError::Integrity(
            "new slash acknowledgement changed; PATCH refused".into(),
        )
        .into());
    }
    let key = cdr_store::new_reply::acknowledgement_key(&record)?;
    let hash = hex::encode(Sha256::digest(result.text.as_bytes()));
    match delivery_receipt::begin(db, &key, &hash)? {
        ReceiptState::Delivered(_) => return Ok(true),
        ReceiptState::New => {}
        state => {
            return Err(StoreError::Integrity(format!(
                "new slash acknowledgement retained without retry: {state:?}"
            ))
            .into());
        }
    }
    // Never turn an expired interaction into a different channel POST.
    api.update_initial_response(&work.interaction_token, &result.text)
        .await
        .map_err(|source| {
            InteractionWorkerError::Delivery(DeliveryFailure {
                part: 1,
                total_parts: 1,
                attempts: 1,
                source,
            })
        })?;
    if !delivery_receipt::confirm(
        db,
        &key,
        &format!("initial-response/{}", work.interaction_id),
    )? {
        return Err(StoreError::Integrity(
            "new slash acknowledgement accepted but confirmation commit failed".into(),
        )
        .into());
    }
    Ok(true)
}
