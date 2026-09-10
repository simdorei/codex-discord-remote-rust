use std::collections::BTreeSet;

use cdr_discord::interaction::RoutedWork;
use cdr_discord::interaction_access::{InteractionAccessPolicy, route_interaction};
use serde_json::json;
use twilight_model::{
    application::interaction::Interaction, channel::message::MessageFlags,
    http::interaction::InteractionResponseType,
};

fn interaction(channel: u64, user: u64, command: &str) -> Interaction {
    serde_json::from_value(json!({
        "application_id":"2",
        "authorizing_integration_owners":{},
        "channel_id":channel.to_string(),
        "data":{"id":"3","name":command,"type":1},
        "entitlements":[],
        "id":"4",
        "locale":"en-US",
        "token":"interaction-token",
        "type":2,
        "user":{
            "avatar":null,
            "bot":false,
            "discriminator":"0001",
            "id":user.to_string(),
            "username":"tester"
        },
        "version":1
    }))
    .unwrap()
}

#[test]
fn allowed_slash_interaction_is_deferred_and_keeps_transport_identity() {
    let input = interaction(10, 20, "help");
    let policy = InteractionAccessPolicy {
        allowed_channel_ids: BTreeSet::from([10]),
        allowed_user_ids: BTreeSet::from([20]),
        mirrored_channel_ids: BTreeSet::new(),
        allow_all_channels: false,
    };

    let routed = route_interaction(&input, &policy, false);

    assert_eq!(routed.interaction_id.get(), 4);
    assert_eq!(routed.channel_id.unwrap().get(), 10);
    assert_eq!(routed.user_id.unwrap().get(), 20);
    assert_eq!(routed.token, "interaction-token");
    assert_eq!(
        routed.initial_response.kind,
        InteractionResponseType::DeferredChannelMessageWithSource
    );
    assert!(matches!(routed.work, Some(RoutedWork::Slash(_))));
}

#[test]
fn mirrored_channels_are_allowed_and_an_empty_user_allowlist_means_all_users() {
    let input = interaction(11, 99, "help");
    let policy = InteractionAccessPolicy {
        allowed_channel_ids: BTreeSet::from([10]),
        allowed_user_ids: BTreeSet::new(),
        mirrored_channel_ids: BTreeSet::from([11]),
        allow_all_channels: false,
    };

    assert!(route_interaction(&input, &policy, false).work.is_some());
}

#[test]
fn denied_and_malformed_interactions_receive_immediate_ephemeral_errors() {
    let denied = route_interaction(
        &interaction(12, 20, "help"),
        &InteractionAccessPolicy {
            allowed_channel_ids: BTreeSet::from([10]),
            allowed_user_ids: BTreeSet::from([20]),
            mirrored_channel_ids: BTreeSet::new(),
            allow_all_channels: false,
        },
        false,
    );
    assert!(denied.work.is_none());
    assert_eq!(
        denied.initial_response.kind,
        InteractionResponseType::ChannelMessageWithSource
    );
    let data = denied.initial_response.data.unwrap();
    assert_eq!(data.flags, Some(MessageFlags::EPHEMERAL));
    assert!(data.content.unwrap().contains("channel"));

    let malformed = route_interaction(
        &interaction(10, 20, "unknown"),
        &InteractionAccessPolicy {
            allowed_channel_ids: BTreeSet::from([10]),
            allowed_user_ids: BTreeSet::new(),
            mirrored_channel_ids: BTreeSet::new(),
            allow_all_channels: false,
        },
        false,
    );
    assert!(malformed.work.is_none());
    assert!(
        malformed
            .initial_response
            .data
            .unwrap()
            .content
            .unwrap()
            .contains("unknown")
    );
}
