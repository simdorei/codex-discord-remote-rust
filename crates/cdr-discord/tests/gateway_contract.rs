use cdr_discord::gateway::{gateway_event_flags, gateway_intents};
use twilight_gateway::{EventTypeFlags, Intents};

#[test]
fn gateway_intents_match_discord_py_default_and_optional_message_content() {
    let without_content = gateway_intents(false);
    let with_content = gateway_intents(true);
    assert_eq!(without_content.bits(), 53_575_421);
    assert_eq!(with_content.bits(), 53_608_189);
    assert!(!without_content.contains(Intents::MESSAGE_CONTENT));
    assert!(with_content.contains(Intents::MESSAGE_CONTENT));
}

#[test]
fn gateway_parses_every_event_for_existing_debug_and_recovery_contracts() {
    let flags = gateway_event_flags();
    assert_eq!(flags, EventTypeFlags::all());
    assert!(flags.contains(EventTypeFlags::READY));
    assert!(flags.contains(EventTypeFlags::MESSAGE_CREATE));
    assert!(flags.contains(EventTypeFlags::INTERACTION_CREATE));
}
