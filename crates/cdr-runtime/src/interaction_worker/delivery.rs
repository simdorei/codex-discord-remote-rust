use cdr_discord::delivery::{DeliveryFailure, DeliveryPolicy, deliver_text_indexed};
use cdr_discord::http::{DiscordHttp, DiscordHttpError};

use crate::discord_dispatch::InboundInteractionWork;
use crate::discord_dispatch::delivery_identity::interaction_delivery_key;

pub(super) async fn deliver_interaction_text_idempotent(
    api: &DiscordHttp,
    work: &InboundInteractionWork,
    content: &str,
    domain: &str,
) -> Result<usize, DeliveryFailure<DiscordHttpError>> {
    let logical_key = interaction_delivery_key(work.interaction_id);
    let policy = DeliveryPolicy::default();
    deliver_text_indexed(content, &policy, |part, chunk| {
        let logical_key = logical_key.as_str();
        async move {
            if part == 0 {
                api.update_initial_response(&work.interaction_token, &chunk)
                    .await
            } else {
                api.send_idempotent_message(work.channel_id, &chunk, &[], domain, logical_key, part)
                    .await
            }
        }
    })
    .await
}
