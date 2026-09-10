use super::{MessageClassification, admit_message_candidate_at, classify_gateway_message};
use crate::config::{CliOptions, RuntimeConfig};
use serde_json::json;
use std::{collections::BTreeMap, time::SystemTime};

#[test]
fn preclassified_mentions_exempt_messages_cannot_execute_ask_after_arm_is_consumed() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let mut config = RuntimeConfig::from_map(
        &BTreeMap::from([
            ("DISCORD_BOT_TOKEN".into(), "fixture-token".into()),
            ("DISCORD_ALLOW_ALL_CHANNELS".into(), "1".into()),
        ]),
        CliOptions::default(),
    )
    .unwrap();
    config.plain_ask_mention_user_ids.insert(777);
    let policy = cdr_discord::interaction_access::InteractionAccessPolicy {
        allow_all_channels: true,
        ..Default::default()
    };
    let classify = |id: u64, content: &str| {
        let message=serde_json::from_value(json!({
            "attachments":[],"author":{"avatar":null,"bot":false,"discriminator":"0001","id":"20","username":"fixture"},
            "channel_id":"99","content":content,"edited_timestamp":null,"embeds":[],"id":id.to_string(),
            "mention_everyone":false,"mention_roles":[],"mentions":[],"pinned":false,
            "timestamp":"2020-02-02T02:02:02.020000+00:00","tts":false,"type":0
        })).unwrap();
        let MessageClassification::Candidate(candidate) =
            classify_gateway_message(message, &db, &config, &policy, None).unwrap()
        else {
            panic!("candidate")
        };
        candidate
    };
    drop(admit_message_candidate_at(classify(30, "!new"), SystemTime::now()).unwrap());
    // The history runner classifies the whole page before admitting its rows.
    let candidates = [classify(31, "first"), classify(32, "second")];
    let plans = candidates
        .into_iter()
        .map(|candidate| {
            admit_message_candidate_at(candidate, SystemTime::now())
                .unwrap()
                .unwrap()
                .into_processing_parts(&db)
                .unwrap()
                .frozen_plan
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        plans
            .iter()
            .filter(|p| matches!(
                p,
                crate::message_plan::MessagePlan::Execute(
                    crate::command_plan::CommandAction::New { .. }
                )
            ))
            .count(),
        1
    );
    assert_eq!(
        plans
            .iter()
            .filter(|p| matches!(
                p,
                crate::message_plan::MessagePlan::Execute(
                    crate::command_plan::CommandAction::Ask { .. }
                )
            ))
            .count(),
        0,
        "a consumed reservation must not grant a normal Ask mention exemption"
    );
}

#[test]
fn bare_new_next_message_does_not_require_an_extra_mention() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let mut config = RuntimeConfig::from_map(
        &BTreeMap::from([
            ("DISCORD_BOT_TOKEN".into(), "fixture-token".into()),
            ("DISCORD_ALLOW_ALL_CHANNELS".into(), "1".into()),
        ]),
        CliOptions::default(),
    )
    .unwrap();
    config.plain_ask_mention_user_ids.insert(777);
    let policy = cdr_discord::interaction_access::InteractionAccessPolicy {
        allow_all_channels: true,
        ..Default::default()
    };
    for (id, content) in [(30, "!new"), (31, "첫 요청")] {
        let message=serde_json::from_value(json!({
            "attachments":[],"author":{"avatar":null,"bot":false,"discriminator":"0001","id":"20","username":"fixture"},
            "channel_id":"99","content":content,"edited_timestamp":null,"embeds":[],"id":id.to_string(),
            "mention_everyone":false,"mention_roles":[],"mentions":[],"pinned":false,
            "timestamp":"2020-02-02T02:02:02.020000+00:00","tts":false,"type":0
        })).unwrap();
        let MessageClassification::Candidate(candidate) =
            classify_gateway_message(message, &db, &config, &policy, None).unwrap()
        else {
            panic!("pending !new prompt must not be ignored")
        };
        let admitted = admit_message_candidate_at(candidate, SystemTime::now())
            .unwrap()
            .unwrap();
        let parts = admitted.into_processing_parts(&db).unwrap();
        if id == 31 {
            assert!(matches!(
                parts.frozen_plan,
                Ok(crate::message_plan::MessagePlan::Execute(
                    crate::command_plan::CommandAction::New { .. }
                ))
            ));
        }
    }
}

#[test]
fn production_message_admission_envelope_can_handoff_new_prompt() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let config = RuntimeConfig::from_map(
        &BTreeMap::from([
            ("DISCORD_BOT_TOKEN".into(), "fixture-token".into()),
            ("DISCORD_ALLOW_ALL_CHANNELS".into(), "1".into()),
        ]),
        CliOptions::default(),
    )
    .unwrap();
    let message=serde_json::from_value(json!({
        "attachments":[],"author":{"avatar":null,"bot":false,"discriminator":"0001","id":"20","username":"fixture"},
        "channel_id":"99","content":"!new 새 작업","edited_timestamp":null,"embeds":[],"id":"30",
        "mention_everyone":false,"mention_roles":[],"mentions":[],"pinned":false,
        "timestamp":"2020-02-02T02:02:02.020000+00:00","tts":false,"type":0
    })).unwrap();
    let policy = cdr_discord::interaction_access::InteractionAccessPolicy {
        allow_all_channels: true,
        ..Default::default()
    };
    let MessageClassification::Candidate(candidate) =
        classify_gateway_message(message, &db, &config, &policy, None).unwrap()
    else {
        panic!("new is actionable")
    };
    let admitted = admit_message_candidate_at(candidate, SystemTime::now())
        .unwrap()
        .unwrap();
    let mut parts = admitted.into_processing_parts(&db).unwrap();
    parts.custody.begin(None).unwrap();
    let crate::message_plan::MessagePlan::Execute(crate::command_plan::CommandAction::New {
        prompt,
    }) = parts.frozen_plan.unwrap()
    else {
        panic!("new plan")
    };
    super::new_handoff_tests::handoff(&db, "message:30", &prompt);
    cdr_store::ingress::record_result(&db, "message:30", &json!({"waits_for_final":true}), 5.0)
        .unwrap();
    parts.custody.finish().unwrap();
}

pub(crate) fn handoff(db: &std::path::Path, key: &str, prompt: &str) {
    use cdr_store::ingress;
    let original = ingress::get(db, key).unwrap().unwrap().payload;
    assert_eq!(prompt, "새 작업");
    assert!(ingress::begin_thread_start(db, key, 1, 2.0).unwrap());
    ingress::record_created_thread(db, key, 1, "new-thread", 3.0).unwrap();
    cdr_store::mapping::upsert_thread(db, "new-thread", "project", "new", 99, 100, 3.0).unwrap();
    let result = cdr_store::prompt_intake::admit_prompt_intake_with_ingress(
        db,
        cdr_store::prompt_intake::NewPromptIntake {
            job_id: "first-prompt",
            target_thread_id: "new-thread",
            channel_id: 100,
            owner_user_id: Some(20),
            discord_message_id: Some(30),
            raw_prompt: prompt,
            auto_queue_when_busy: true,
            require_current_mirror: true,
            created_at: 4.0,
        },
        key,
        1,
    )
    .unwrap();
    assert_eq!(result.intake.channel_id, 100);
    let saved = ingress::get(db, key).unwrap().unwrap();
    assert_eq!(saved.payload, original);
    assert_eq!(saved.owner_id.as_deref(), Some("first-prompt"));
}
