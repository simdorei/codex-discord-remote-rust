use cdr_discord::responses::{interaction_message, invalid_interaction_message};
use twilight_model::{channel::message::MessageFlags, http::interaction::InteractionResponseType};

#[test]
fn permission_denials_are_immediate_ephemeral_responses() {
    let response = interaction_message("Not allowed in this channel.", true);
    assert_eq!(
        response.kind,
        InteractionResponseType::ChannelMessageWithSource
    );
    let data = response.data.unwrap();
    assert_eq!(
        data.content.as_deref(),
        Some("Not allowed in this channel.")
    );
    assert_eq!(data.flags, Some(MessageFlags::EPHEMERAL));
    assert!(data.allowed_mentions.is_some());
}

#[test]
fn malformed_interactions_expose_the_actual_public_safe_error() {
    let response = invalid_interaction_message("unknown command");
    let data = response.data.unwrap();
    assert_eq!(
        data.content.as_deref(),
        Some("Discord interaction rejected: unknown command")
    );
    assert_eq!(data.flags, Some(MessageFlags::EPHEMERAL));
}
