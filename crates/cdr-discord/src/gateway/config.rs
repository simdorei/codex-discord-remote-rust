use twilight_gateway::{EventTypeFlags, Intents};

#[must_use]
pub fn gateway_intents(message_content: bool) -> Intents {
    let defaults = Intents::GUILDS
        | Intents::GUILD_MODERATION
        | Intents::GUILD_EMOJIS_AND_STICKERS
        | Intents::GUILD_INTEGRATIONS
        | Intents::GUILD_WEBHOOKS
        | Intents::GUILD_INVITES
        | Intents::GUILD_VOICE_STATES
        | Intents::GUILD_MESSAGES
        | Intents::GUILD_MESSAGE_REACTIONS
        | Intents::GUILD_MESSAGE_TYPING
        | Intents::DIRECT_MESSAGES
        | Intents::DIRECT_MESSAGE_REACTIONS
        | Intents::DIRECT_MESSAGE_TYPING
        | Intents::GUILD_SCHEDULED_EVENTS
        | Intents::AUTO_MODERATION_CONFIGURATION
        | Intents::AUTO_MODERATION_EXECUTION
        | Intents::GUILD_MESSAGE_POLLS
        | Intents::DIRECT_MESSAGE_POLLS;
    if message_content {
        defaults | Intents::MESSAGE_CONTENT
    } else {
        defaults
    }
}

#[must_use]
pub fn gateway_event_flags() -> EventTypeFlags {
    EventTypeFlags::all()
}
