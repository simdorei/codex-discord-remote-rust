use twilight_model::{
    channel::message::{AllowedMentions, MessageFlags},
    http::interaction::{InteractionResponse, InteractionResponseData, InteractionResponseType},
};

use crate::text::{DISCORD_MAX_LEN, fit_single_message};

#[must_use]
pub fn interaction_message(content: &str, ephemeral: bool) -> InteractionResponse {
    InteractionResponse {
        kind: InteractionResponseType::ChannelMessageWithSource,
        data: Some(InteractionResponseData {
            allowed_mentions: Some(AllowedMentions::default()),
            content: Some(fit_single_message(content, DISCORD_MAX_LEN)),
            flags: ephemeral.then_some(MessageFlags::EPHEMERAL),
            ..InteractionResponseData::default()
        }),
    }
}

#[must_use]
pub fn invalid_interaction_message(reason: &str) -> InteractionResponse {
    interaction_message(&format!("Discord interaction rejected: {reason}"), true)
}

#[must_use]
pub fn autocomplete_response() -> InteractionResponse {
    InteractionResponse {
        kind: InteractionResponseType::ApplicationCommandAutocompleteResult,
        data: Some(InteractionResponseData {
            choices: Some(Vec::new()),
            ..InteractionResponseData::default()
        }),
    }
}
