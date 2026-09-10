use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::SystemTime;

use cdr_discord::interaction_access::InteractionAccessPolicy;
use cdr_store::processed::is_processed;
use twilight_model::channel::{Attachment, Message};
use twilight_model::id::{Id, marker::AttachmentMarker};

use super::{
    MessageCandidate, MessageClassification, classify_gateway_message,
    classify_gateway_message_with,
};
use crate::config::{CliOptions, RuntimeConfig};
use crate::message_plan::MessagePlanError;
use crate::message_worker::{AdmittedMessage, admit_message_candidate_at};

trait AmbiguousIfClone<Marker> {
    fn marker() {}
}

impl<T: ?Sized> AmbiguousIfClone<()> for T {}

struct CloneMarker;

impl<T: Clone> AmbiguousIfClone<CloneMarker> for T {}

const _: fn() = || {
    let _ = <MessageCandidate as AmbiguousIfClone<_>>::marker;
    let _ = <AdmittedMessage as AmbiguousIfClone<_>>::marker;
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

fn allow_all() -> InteractionAccessPolicy {
    InteractionAccessPolicy {
        allow_all_channels: true,
        ..InteractionAccessPolicy::default()
    }
}

#[test]
fn explicit_pro_is_not_admitted_as_a_drain_reply_candidate() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    cdr_store::mapping::upsert_thread(&db, "original", "project", "title", 100, 2, 1.0).unwrap();
    for (raw, eligible) in [
        ("!pro 확인", false),
        ("!pro review 검수", false),
        ("답변입니다", true),
        (
            "[codex-reply:0123456789abcdef0123456789abcdef] !pro는 문구",
            true,
        ),
    ] {
        let MessageClassification::Candidate(candidate) =
            classify_gateway_message(message(100, raw), &db, &config(), &allow_all(), None)
                .unwrap()
        else {
            panic!("candidate");
        };
        assert_eq!(candidate.is_pending_reply_candidate(), eligible, "{raw}");
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

fn assert_ignored(
    database: &Path,
    message: Message,
    config: &RuntimeConfig,
    policy: &InteractionAccessPolicy,
    bot_user_id: Option<u64>,
    expected_reason: &str,
) {
    let persisted_id = i64::try_from(message.id.get()).expect("test ID fits SQLite");
    let classification = classify_gateway_message(message, database, config, policy, bot_user_id)
        .expect("read-only classification succeeds");
    let mut effects = 0;
    let actual_reason = match classification {
        MessageClassification::Ignore(ignored) => ignored.into_log_parts().0,
        MessageClassification::Candidate(candidate) => {
            if admit_message_candidate_at(candidate, SystemTime::now())
                .expect("unexpected candidate admission succeeds")
                .is_some()
            {
                effects += 1;
            }
            "unexpected_candidate"
        }
    };
    assert_eq!(actual_reason, expected_reason);
    assert_eq!(
        effects, 0,
        "ignored input must not reach an effect callback"
    );
    assert!(
        !is_processed(database, persisted_id).expect("read processed-message state"),
        "ignored input must not create a permanent replay row"
    );
}

#[test]
fn mcg_01_every_ignore_reason_leaves_zero_processed_rows_and_effects() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");
    let base_config = config();
    let base_policy = allow_all();

    let mut disabled = base_config.clone();
    disabled.enable_message_content = false;
    assert_ignored(
        &database,
        message(101, "hello"),
        &disabled,
        &base_policy,
        None,
        "message_content_disabled",
    );
    assert_ignored(
        &database,
        message(102, "hello"),
        &base_config,
        &InteractionAccessPolicy::default(),
        None,
        "channel_not_allowed",
    );
    let mut denied_user = base_policy.clone();
    denied_user.allowed_user_ids = BTreeSet::from([999]);
    assert_ignored(
        &database,
        message(103, "hello"),
        &base_config,
        &denied_user,
        None,
        "user_not_allowed",
    );
    assert_ignored(
        &database,
        message(104, "hello"),
        &base_config,
        &base_policy,
        Some(3),
        "self_authored",
    );
    let mut bot = message(105, "hello");
    bot.author.bot = true;
    assert_ignored(
        &database,
        bot,
        &base_config,
        &base_policy,
        Some(9),
        "bot_author_without_bridge_mention",
    );
    let mut mention_required = base_config.clone();
    mention_required.plain_ask_mention_user_ids = BTreeSet::from([42]);
    assert_ignored(
        &database,
        message(106, "hello"),
        &mention_required,
        &base_policy,
        None,
        "required_mention_missing",
    );
    assert_ignored(
        &database,
        message(107, ""),
        &base_config,
        &base_policy,
        None,
        "empty_content",
    );
}

#[test]
fn mcg_02_ignored_attachment_performs_no_directory_or_http_stage() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");
    let attachment_root = temp.path().join("attachments-must-not-exist");
    let mut attached = message(108, "hello");
    attached.attachments.push(Attachment {
        content_type: Some("text/plain".into()),
        ephemeral: false,
        duration_secs: None,
        filename: "never-downloaded.txt".into(),
        flags: None,
        description: None,
        height: None,
        id: Id::<AttachmentMarker>::new(1),
        proxy_url: "http://127.0.0.1:9/never".into(),
        size: 1,
        title: None,
        url: "http://127.0.0.1:9/never".into(),
        waveform: None,
        width: None,
    });
    assert_ignored(
        &database,
        attached,
        &config(),
        &InteractionAccessPolicy::default(),
        None,
        "channel_not_allowed",
    );
    assert!(!attachment_root.exists());
}

#[test]
fn mcg_03_planner_runs_once_and_its_error_is_frozen_in_the_candidate() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");
    let input = message(109, "!broken");
    let calls = Cell::new(0);
    let classification = classify_gateway_message_with(
        input,
        &database,
        &config(),
        &allow_all(),
        None,
        |incoming| {
            calls.set(calls.get() + 1);
            assert_eq!(incoming.content, "!broken");
            Err(MessagePlanError::UnsupportedPrefix("sentinel"))
        },
    )
    .expect("classification succeeds");
    let MessageClassification::Candidate(candidate) = classification else {
        panic!("planning errors must remain admission candidates");
    };
    let parts = candidate.into_admission_parts();
    assert_eq!(calls.get(), 1);
    assert!(matches!(
        parts.frozen_plan,
        Err(MessagePlanError::UnsupportedPrefix("sentinel"))
    ));
    let owned_message: Box<Message> = parts.message;
    assert_eq!(owned_message.id.get(), 109);
}
