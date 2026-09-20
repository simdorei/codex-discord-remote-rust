use std::cell::Cell;

use serde_json::json;
use tokio::time::Instant;
use twilight_model::{
    gateway::{
        event::Event,
        payload::incoming::{InteractionCreate, MessageCreate, Ready},
    },
    oauth::{ApplicationFlags, PartialApplication},
    user::CurrentUser,
};

use super::ingress::{
    GatewayIngress, GatewayIngressConfig, InteractionIngressTag, PublishOutcome, UnavailableReason,
};

#[test]
fn force_restart_bypasses_a_full_normal_message_queue() {
    let (ingress, mut receivers) = GatewayIngress::new(config(1, 1, 1)).unwrap();
    assert!(matches!(
        ingress.publish(message_event(1), Instant::now()),
        PublishOutcome::MessageAccepted { .. }
    ));
    let Event::MessageCreate(mut force) = message_event(2) else {
        unreachable!()
    };
    force.content = "!restart_codex force".into();
    assert!(matches!(
        ingress.publish(Event::MessageCreate(force), Instant::now()),
        PublishOutcome::MessageAccepted { .. }
    ));
    assert_eq!(
        receivers
            .emergency_messages
            .try_recv()
            .unwrap()
            .event
            .id
            .get(),
        2
    );
    assert_eq!(receivers.messages.try_recv().unwrap().event.id.get(), 1);
}

pub(super) fn config(normal: usize, reserved: usize, messages: usize) -> GatewayIngressConfig {
    GatewayIngressConfig {
        interaction_capacity: normal,
        reserved_interaction_capacity: reserved,
        message_capacity: messages,
        receive_error_capacity: 16,
    }
}

pub(super) fn interaction_event(id: u64) -> Event {
    let interaction: InteractionCreate = serde_json::from_value(json!({
        "application_id": "200",
        "authorizing_integration_owners": {},
        "id": id.to_string(),
        "token": "offline-token",
        "type": 1
    }))
    .expect("minimal ping interaction");
    Event::InteractionCreate(Box::new(interaction))
}

pub(super) fn message_event(id: u64) -> Event {
    message_event_at(id, 300, "2026-01-01T00:00:00.000000+00:00")
}

pub(super) fn message_event_at(id: u64, channel_id: u64, timestamp: &str) -> Event {
    let message: MessageCreate = serde_json::from_value(json!({
        "id": id.to_string(),
        "channel_id": channel_id.to_string(),
        "author": {
            "id": "400",
            "username": "offline-user",
            "discriminator": "0001",
            "avatar": null
        },
        "content": "offline message",
        "timestamp": timestamp,
        "edited_timestamp": null,
        "tts": false,
        "mention_everyone": false,
        "mentions": [],
        "mention_roles": [],
        "attachments": [],
        "embeds": [],
        "pinned": false,
        "type": 0
    }))
    .expect("minimal message create");
    Event::MessageCreate(Box::new(message))
}

pub(super) fn ready_event(user_id: u64, application_id: u64) -> Event {
    Event::Ready(Ready {
        application: PartialApplication {
            flags: ApplicationFlags::empty(),
            id: twilight_model::id::Id::new(application_id),
        },
        guilds: Vec::new(),
        resume_gateway_url: "wss://gateway.discord.gg".into(),
        session_id: "offline-session".into(),
        shard: None,
        user: CurrentUser {
            accent_color: None,
            avatar: None,
            banner: None,
            bot: true,
            discriminator: 0,
            email: None,
            flags: None,
            global_name: None,
            id: twilight_model::id::Id::new(user_id),
            locale: None,
            mfa_enabled: false,
            name: "bot".into(),
            premium_type: None,
            public_flags: None,
            verified: None,
        },
        version: 10,
    })
}

#[test]
fn gi_in_01_message_pressure_never_consumes_or_evicts_interaction_capacity() {
    let (ingress, mut receivers) = GatewayIngress::new(config(1, 1, 1)).expect("valid config");
    let diagnostics = ingress.subscribe_diagnostics();

    let first = ingress.publish(message_event(1), Instant::now());
    let overflow = ingress.publish(message_event(2), Instant::now());
    let interaction = ingress.publish(interaction_event(3), Instant::now());

    assert!(matches!(first, PublishOutcome::MessageAccepted { .. }));
    assert_eq!(
        overflow,
        PublishOutcome::MessageRecoverableGap {
            reason: UnavailableReason::Full,
            tracking: Ok(())
        }
    );
    assert!(matches!(
        interaction,
        PublishOutcome::InteractionAccepted {
            tag: InteractionIngressTag::Normal,
            ..
        }
    ));
    assert_eq!(receivers.messages.try_recv().unwrap().event.id.get(), 1);
    assert_eq!(
        receivers
            .normal_interactions
            .try_recv()
            .unwrap()
            .event
            .id
            .get(),
        3
    );
    assert_eq!(diagnostics.snapshot().hard_dropped_interactions, 0);
    assert_eq!(diagnostics.snapshot().recoverable_message_gaps, 1);
}

#[test]
fn gi_in_02_normal_overflow_uses_exact_reserved_capacity_then_records_hard_drop() {
    let (ingress, mut receivers) = GatewayIngress::new(config(1, 2, 1)).expect("valid config");
    let diagnostics = ingress.subscribe_diagnostics();

    assert!(matches!(
        ingress.publish(interaction_event(10), Instant::now()),
        PublishOutcome::InteractionAccepted {
            tag: InteractionIngressTag::Normal,
            ..
        }
    ));
    for id in [11, 12] {
        assert!(matches!(
            ingress.publish(interaction_event(id), Instant::now()),
            PublishOutcome::InteractionAccepted {
                tag: InteractionIngressTag::Busy,
                ..
            }
        ));
    }
    assert_eq!(
        ingress.publish(interaction_event(13), Instant::now()),
        PublishOutcome::InteractionHardDropped {
            reason: UnavailableReason::Full
        }
    );

    assert_eq!(
        receivers.normal_interactions.try_recv().unwrap().tag,
        InteractionIngressTag::Normal
    );
    assert_eq!(
        receivers.reserved_interactions.try_recv().unwrap().tag,
        InteractionIngressTag::Busy
    );
    assert_eq!(
        receivers.reserved_interactions.try_recv().unwrap().tag,
        InteractionIngressTag::Busy
    );
    assert_eq!(diagnostics.snapshot().hard_dropped_interactions, 1);
    assert_eq!(diagnostics.snapshot().recoverable_message_gaps, 0);
}

#[test]
fn gi_in_03_ready_is_sticky_and_observable_before_callback_when_every_lane_is_full() {
    let (ingress, _receivers) = GatewayIngress::new(config(1, 1, 1)).expect("valid config");
    assert!(matches!(
        ingress.publish(message_event(20), Instant::now()),
        PublishOutcome::MessageAccepted { .. }
    ));
    assert!(matches!(
        ingress.publish(interaction_event(21), Instant::now()),
        PublishOutcome::InteractionAccepted {
            tag: InteractionIngressTag::Normal,
            ..
        }
    ));
    assert!(matches!(
        ingress.publish(interaction_event(22), Instant::now()),
        PublishOutcome::InteractionAccepted {
            tag: InteractionIngressTag::Busy,
            ..
        }
    ));
    let identity = ingress.subscribe_identity();
    let conflict = ingress.subscribe_identity_conflict();
    let callback_ran = Cell::new(false);

    let outcome = ingress.publish_with_observer(ready_event(500, 600), Instant::now(), || {
        assert_eq!(identity.snapshot().unwrap().user_id.get(), 500);
        assert_eq!(identity.snapshot().unwrap().application_id.get(), 600);
        assert!(conflict.snapshot().is_none());
        callback_ran.set(true);
    });

    assert_eq!(outcome, PublishOutcome::Ignored);
    assert!(callback_ran.get());
    assert_eq!(identity.snapshot().unwrap().user_id.get(), 500);
}

#[test]
fn gi_in_04_stop_routes_interactions_to_stopping_reserve_and_messages_to_gap() {
    let (ingress, mut receivers) = GatewayIngress::new(config(2, 1, 1)).expect("valid config");
    let diagnostics = ingress.subscribe_diagnostics();
    ingress.stop_accepting();

    assert!(matches!(
        ingress.publish(interaction_event(30), Instant::now()),
        PublishOutcome::InteractionAccepted {
            tag: InteractionIngressTag::Stopping,
            ..
        }
    ));
    assert_eq!(
        ingress.publish(message_event(31), Instant::now()),
        PublishOutcome::MessageRecoverableGap {
            reason: UnavailableReason::Stopping,
            tracking: Ok(())
        }
    );
    assert_eq!(
        receivers.reserved_interactions.try_recv().unwrap().tag,
        InteractionIngressTag::Stopping
    );
    assert!(receivers.normal_interactions.try_recv().is_err());
    assert_eq!(diagnostics.snapshot().hard_dropped_interactions, 0);
    assert_eq!(diagnostics.snapshot().recoverable_message_gaps, 1);
}
