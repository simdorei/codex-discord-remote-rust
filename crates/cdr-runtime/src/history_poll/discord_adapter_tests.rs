use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, UNIX_EPOCH};

use cdr_discord::interaction_access::InteractionAccessPolicy;
use cdr_store::processed::is_processed;

use super::*;
use crate::config::{CliOptions, RuntimeConfig};

trait AmbiguousIfClone<Marker> {
    fn marker() {}
}

impl<T: ?Sized> AmbiguousIfClone<()> for T {}

struct CloneMarker;

impl<T: Clone> AmbiguousIfClone<CloneMarker> for T {}

const _: fn() = || {
    let _ = <HistoryMessageCandidate as AmbiguousIfClone<_>>::marker;
    let _ = <HistoryAdmittedMessage as AmbiguousIfClone<_>>::marker;
};

fn config() -> RuntimeConfig {
    RuntimeConfig::from_map(
        &BTreeMap::from([
            ("DISCORD_BOT_TOKEN".into(), "test-token".into()),
            ("DISCORD_ALLOW_ALL_CHANNELS".into(), "1".into()),
        ]),
        CliOptions::default(),
    )
    .expect("valid test config")
}

fn policy(channel_id: u64) -> InteractionAccessPolicy {
    InteractionAccessPolicy {
        allowed_channel_ids: BTreeSet::from([channel_id]),
        ..InteractionAccessPolicy::default()
    }
}

fn message(id: u64, author_is_bot: bool) -> Message {
    serde_json::from_value(serde_json::json!({
        "attachments": [],
        "author": {"avatar": null, "bot": author_is_bot, "discriminator": "0001",
            "id": "3", "username": "tester"},
        "channel_id": "2", "content": "hello", "edited_timestamp": null,
        "embeds": [], "id": id.to_string(), "mention_everyone": false,
        "mention_roles": [], "mentions": [], "pinned": false,
        "timestamp": "2020-02-02T02:02:02.020000+00:00", "tts": false, "type": 0
    }))
    .expect("valid history message fixture")
}

#[test]
fn force_restart_from_history_is_never_replayed() {
    for command in [
        "!force_restart",
        "!restart_codex force",
        "!restart_codex --force",
    ] {
        let mut input = message(990, false);
        input.content = command.into();
        let adapted = adapt_discord_history_message_with(input, |_| {
            panic!("historical force command must not reach admission")
        })
        .unwrap();
        assert!(matches!(adapted.kind, HistoryPollItem::Ignore));
    }
}

#[test]
fn dha_01_bot_author_is_ignored_before_classifier_even_when_bridge_is_mentioned() {
    let mut input = message(501, true);
    input.mentions = serde_json::from_value(serde_json::json!([{
        "avatar": null, "bot": true, "discriminator": "0001", "id": "9",
        "public_flags": 0, "username": "bridge"
    }]))
    .expect("valid bridge mention");
    let expected_watermark =
        HistoryWatermark::from_message(input.timestamp.as_micros(), input.id.get());
    let calls = Cell::new(0);

    let adapted = adapt_discord_history_message_with(input, |_| {
        calls.set(calls.get() + 1);
        panic!("bot history must skip mirror lookup and canonical planning")
    })
    .expect("bot history is a successful ignore");

    assert_eq!(adapted.watermark, expected_watermark);
    assert!(matches!(adapted.kind, HistoryPollItem::Ignore));
    assert_eq!(calls.get(), 0);
}

#[test]
fn dha_02_each_user_payload_calls_the_canonical_classifier_exactly_once() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");
    let config = config();
    let policy = policy(2);
    let calls = Cell::new(0);

    let adapted = adapt_discord_history_message_with(message(502, false), |message| {
        calls.set(calls.get() + 1);
        classify_gateway_message(message, &database, &config, &policy, Some(9))
    })
    .expect("user history adapts");

    assert!(matches!(adapted.kind, HistoryPollItem::Candidate(_)));
    assert_eq!(calls.get(), 1);
}

#[test]
fn dha_03_one_owned_policy_snapshot_is_reused_for_the_whole_adapter_cycle() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");
    let config = config();
    let mut source_policy = policy(2);
    let adapter =
        DiscordHistoryMessageAdapter::new(&config, &database, source_policy.clone(), Some(9));
    source_policy.allowed_channel_ids.clear();

    for id in [503, 504] {
        let item = adapter.adapt(message(id, false)).expect("snapshot adapts");
        assert!(matches!(item.kind, HistoryPollItem::Candidate(_)));
    }
    assert!(source_policy.allowed_channel_ids.is_empty());
}

#[test]
fn dha_04_history_and_live_share_the_same_durable_claim() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");
    let config = config();
    let adapter = DiscordHistoryMessageAdapter::new(&config, &database, policy(2), Some(9));
    let observed_at = UNIX_EPOCH + Duration::from_secs(100);

    let first = adapter.adapt(message(505, false)).expect("first adapt");
    let HistoryPollItem::Candidate(first) = first.kind else {
        panic!("user message must be a candidate");
    };
    assert!(matches!(
        claim_candidate_at(first, observed_at).expect("first claim"),
        HistoryClaimOutcome::Won(_)
    ));

    let duplicate = adapter.adapt(message(505, false)).expect("duplicate adapt");
    let HistoryPollItem::Candidate(duplicate) = duplicate.kind else {
        panic!("duplicate remains a candidate before admission");
    };
    assert!(matches!(
        claim_candidate_at(duplicate, observed_at).expect("duplicate claim"),
        HistoryClaimOutcome::Lost
    ));
    assert!(is_processed(&database, 505).expect("read durable claim"));
}
