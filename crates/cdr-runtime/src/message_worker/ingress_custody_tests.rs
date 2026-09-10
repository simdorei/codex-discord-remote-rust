use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use cdr_discord::interaction_access::InteractionAccessPolicy;
use cdr_store::processed::is_processed;
use serde_json::{Value, json};

use super::admission::admit_message_candidate_at;
use super::classification::{MessageClassification, classify_gateway_message};
use crate::config::{CliOptions, RuntimeConfig};

#[test]
fn ig_01_dropping_message_after_claim_preserves_original_request_in_reopened_store() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("mirror.sqlite");
    let config = RuntimeConfig::from_map(
        &BTreeMap::from([
            ("DISCORD_BOT_TOKEN".into(), "fixture-token".into()),
            ("DISCORD_ALLOW_ALL_CHANNELS".into(), "1".into()),
        ]),
        CliOptions::default(),
    )
    .unwrap();
    let message = serde_json::from_value(json!({
        "attachments": [],
        "author": {"avatar":null,"bot":false,"discriminator":"0001","id":"3","username":"fixture"},
        "channel_id":"2", "content":"!new 보존할 원문\nsecond line",
        "edited_timestamp":null,"embeds":[],"id":"801",
        "mention_everyone":false,"mention_roles":[],"mentions":[],"pinned":false,
        "timestamp":"2020-02-02T02:02:02.020000+00:00","tts":false,"type":0
    }))
    .unwrap();
    let policy = InteractionAccessPolicy {
        allow_all_channels: true,
        ..InteractionAccessPolicy::default()
    };
    let MessageClassification::Candidate(candidate) =
        classify_gateway_message(message, &database, &config, &policy, None).unwrap()
    else {
        panic!("the authorized command must enter admission");
    };
    let owning_message =
        admit_message_candidate_at(candidate, UNIX_EPOCH + Duration::from_secs(100))
            .unwrap()
            .expect("first event is admitted");
    drop(owning_message); // Process cancellation before preparation or ActionExecutor.

    assert!(is_processed(&database, 801).unwrap());
    let reopened = rusqlite::Connection::open(&database).unwrap();
    let has_custody: bool = reopened
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='discord_ingress_journal')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        has_custody,
        "IG-1: an ID-only claim must not outlive the request contents"
    );
    let (actor, channel, payload): (i64, i64, String) = reopened
        .query_row(
            "SELECT owner_user_id, channel_id, payload_json FROM discord_ingress_journal WHERE ingress_id = 'message:801'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("claimed event has recoverable original contents");
    assert_eq!((actor, channel), (3, 2));
    let payload: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload["content"], "!new 보존할 원문\nsecond line");
    assert_eq!(
        payload["plan"]["Execute"]["New"]["prompt"],
        "보존할 원문\nsecond line"
    );
}
