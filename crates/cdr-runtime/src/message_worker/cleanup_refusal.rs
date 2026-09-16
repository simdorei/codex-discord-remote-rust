use super::{
    MessageContext, MessageWorkerError, custody,
    reply_delivery::{MessageReplyKind, send_reply_once},
};
use crate::{action_executor::ActionError, queue_runner::TurnBackend};
use cdr_discord::http::DiscordHttp;
use twilight_model::channel::Message;

pub(super) async fn deliver<B: TurnBackend>(
    message: &Message,
    context: &MessageContext<'_, B>,
    api: &DiscordHttp,
    key: &str,
    error: ActionError,
) -> Result<bool, MessageWorkerError> {
    let Some(refusal) = crate::cleanup_refusal::from_action_error(&error) else {
        return Err(error.into());
    };
    cdr_store::ingress::record_result(
        context.executor.mirror_db(),
        key,
        &refusal.outcome(),
        custody::now()?,
    )?;
    send_reply_once(
        context.executor.mirror_db(),
        api,
        message.channel_id,
        message.id,
        MessageReplyKind::ErrorReport,
        &refusal.message(),
        &[],
    )
    .await
    .map_err(|error| {
        crate::cleanup_refusal::NotificationFailure::delivery(
            context.executor.mirror_db(),
            key,
            error,
        )
    })?;
    Ok(true)
}
