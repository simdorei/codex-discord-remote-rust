use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use cdr_discord::interaction_access::InteractionAccessPolicy;
use cdr_store::processed::is_processed;
use twilight_model::channel::Message;

use super::{MessageAdmissionError, admit_message_candidate_at};
use crate::config::{CliOptions, RuntimeConfig};
use crate::message_plan::{MessagePlan, MessagePlanError};
use crate::message_worker::classification::{MessageClassification, classify_gateway_message};

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

fn policy() -> InteractionAccessPolicy {
    InteractionAccessPolicy {
        allow_all_channels: true,
        ..InteractionAccessPolicy::default()
    }
}

fn message(id: u64, content: &str) -> Message {
    serde_json::from_value(serde_json::json!({
        "attachments": [],
        "author": {
            "avatar": null,
            "bot": false,
            "discriminator": "0001",
            "id": "3",
            "username": "tester"
        },
        "channel_id": "2",
        "content": content,
        "edited_timestamp": null,
        "embeds": [],
        "id": id.to_string(),
        "mention_everyone": false,
        "mention_roles": [],
        "mentions": [],
        "pinned": false,
        "timestamp": "2020-02-02T02:02:02.020000+00:00",
        "tts": false,
        "type": 0
    }))
    .expect("valid gateway message fixture")
}

fn candidate(
    message: Message,
    database: &std::path::Path,
    config: &RuntimeConfig,
) -> super::MessageCandidate {
    match classify_gateway_message(message, database, config, &policy(), None)
        .expect("classification succeeds")
    {
        MessageClassification::Candidate(candidate) => candidate,
        MessageClassification::Ignore(_) => panic!("test input must be a candidate"),
    }
}

fn seen_at(database: &std::path::Path, message_id: i64) -> f64 {
    rusqlite::Connection::open(database)
        .expect("open test database")
        .query_row(
            "SELECT seen_at FROM discord_processed_messages WHERE message_id = ?",
            [message_id],
            |row| row.get(0),
        )
        .expect("read exact processed row")
}

#[test]
fn mca_01_respond_and_planning_error_are_claimed_before_their_effect() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");
    let observed_at = UNIX_EPOCH + Duration::from_secs(100);
    let mut mention_config = config();
    mention_config.plain_ask_mention_user_ids = BTreeSet::from([42]);
    let mut response = message(201, "<@42>");
    response.mentions.push(
        serde_json::from_value(serde_json::json!({
            "avatar": null,
            "bot": false,
            "discriminator": "0001",
            "id": "42",
            "username": "mentioned",
            "public_flags": 0
        }))
        .expect("valid mention fixture"),
    );

    let admitted =
        admit_message_candidate_at(candidate(response, &database, &mention_config), observed_at)
            .expect("response claim succeeds")
            .expect("response is admitted");
    let response_parts = admitted
        .into_processing_parts(&database)
        .expect("response token matches its database");
    assert!(matches!(
        response_parts.frozen_plan,
        Ok(MessagePlan::Respond(_))
    ));
    assert!(is_processed(&database, 201).expect("read response claim"));

    let malformed = message(202, "!not-a-command");
    let admitted =
        admit_message_candidate_at(candidate(malformed, &database, &config()), observed_at)
            .expect("planning-error claim succeeds")
            .expect("planning error is admitted");
    let error_parts = admitted
        .into_processing_parts(&database)
        .expect("error token matches its database");
    assert!(matches!(
        error_parts.frozen_plan,
        Err(MessagePlanError::Prefix(_))
    ));
    assert!(is_processed(&database, 202).expect("read planning-error claim"));
}

#[test]
fn mca_02_redelivery_suppresses_the_processing_callback() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");
    let effects = AtomicUsize::new(0);
    for second in [100, 101] {
        let admitted = admit_message_candidate_at(
            candidate(message(203, "hello"), &database, &config()),
            UNIX_EPOCH + Duration::from_secs(second),
        )
        .expect("admission succeeds");
        if let Some(admitted) = admitted {
            let _ = admitted
                .into_processing_parts(&database)
                .expect("token matches its database");
            effects.fetch_add(1, Ordering::SeqCst);
        }
    }
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert!(is_processed(&database, 203).expect("read durable claim"));
}

#[test]
fn mca_03_database_affinity_mismatch_is_fatal_before_effects() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database_a = temp.path().join("a.sqlite");
    let database_b = temp.path().join("b.sqlite");
    let admitted = admit_message_candidate_at(
        candidate(message(204, "hello"), &database_a, &config()),
        UNIX_EPOCH + Duration::from_secs(100),
    )
    .expect("database A claim succeeds")
    .expect("database A receives the token");
    let effects = AtomicUsize::new(0);
    let result = admitted.into_processing_parts(&database_b);
    if result.is_ok() {
        effects.fetch_add(1, Ordering::SeqCst);
    }

    assert!(matches!(
        result,
        Err(MessageAdmissionError::DatabaseMismatch { .. })
    ));
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    assert!(is_processed(&database_a, 204).expect("read database A"));
    assert!(!is_processed(&database_b, 204).expect("read database B"));
}

#[test]
fn mca_04_downstream_failure_and_success_do_not_change_the_claim_row() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");
    let admitted = admit_message_candidate_at(
        candidate(message(205, "fail later"), &database, &config()),
        UNIX_EPOCH + Duration::from_secs(100),
    )
    .expect("first admission succeeds")
    .expect("first delivery is admitted");
    let _ = admitted
        .into_processing_parts(&database)
        .expect("token matches before simulated downstream failure");
    assert!(
        admit_message_candidate_at(
            candidate(message(205, "fail later"), &database, &config()),
            UNIX_EPOCH + Duration::from_secs(200),
        )
        .expect("redelivery lookup succeeds")
        .is_none()
    );
    assert!((seen_at(&database, 205) - 100.0).abs() < f64::EPSILON);

    let admitted = admit_message_candidate_at(
        candidate(message(206, "succeed later"), &database, &config()),
        UNIX_EPOCH + Duration::from_mins(5),
    )
    .expect("success-path admission succeeds")
    .expect("success-path delivery is admitted");
    let _ = admitted
        .into_processing_parts(&database)
        .expect("token matches before simulated downstream success");
    assert!((seen_at(&database, 206) - 300.0).abs() < f64::EPSILON);
}
