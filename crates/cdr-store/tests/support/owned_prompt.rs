use cdr_store::{
    dead_generation, delivery, delivery_receipt, ingress, mapping, prompt_intake, queue,
};
use std::path::Path;

pub fn request() -> ingress::NewIngress {
    ingress::NewIngress {
        ingress_id: "message:501".into(),
        kind: ingress::IngressKind::Message,
        event_id: Some(501),
        application_id: None,
        channel_id: 31,
        owner_user_id: 42,
        source_message_id: Some(501),
        payload: serde_json::json!({"content":"preserve original prompt"}),
        target_thread_id: Some("target".into()),
        canonical_owner: None,
        now: 2.0,
    }
}

pub fn queued(path: &Path) {
    queued_for(path, "target", "project", 90);
}

pub fn queued_for(path: &Path, target: &str, project: &str, parent: i64) {
    mapping::upsert_thread(path, target, project, "title", parent, 31, 1.0).unwrap();
    dead_generation::activate_runtime(path, "runtime-a").unwrap();
    let mut input = request();
    input.target_thread_id = Some(target.into());
    ingress::admit(path, &input).unwrap();
    ingress::begin_execution(path, "message:501", "processing", Some(target), 2.0).unwrap();
    prompt_intake::admit_prompt_intake(
        path,
        prompt_intake::NewPromptIntake {
            job_id: "owned-job",
            target_thread_id: target,
            channel_id: 31,
            owner_user_id: Some(42),
            discord_message_id: Some(501),
            raw_prompt: "preserve original prompt",
            auto_queue_when_busy: true,
            require_current_mirror: true,
            created_at: 3.0,
        },
    )
    .unwrap();
    let claim = prompt_intake::try_claim_prompt_intake(path, "owned-job", 4.0, 100.0)
        .unwrap()
        .unwrap();
    prompt_intake::promote_prompt_intake_to_queue(
        path,
        &claim,
        queue::NewQueueJob {
            job_id: "owned-job",
            target_thread_id: target,
            channel_id: 31,
            owner_user_id: Some(42),
            discord_message_id: Some(501),
            app_server_generation: 1,
            prompt: "preserve original prompt",
            queued: true,
            ack_sent: true,
            created_at: 4.0,
        },
        5.0,
    )
    .unwrap();
    ingress::record_result(
        path,
        "message:501",
        &serde_json::json!({
            "response":"In progress", "waits_for_final":true,
        }),
        6.0,
    )
    .unwrap();
    ingress::confirm(path, "message:501", 7.0).unwrap();
}

pub fn final_pending(path: &Path) {
    queued(path);
    finish_turn(path);
}

fn finish_turn(path: &Path) {
    queue::begin_attempt(path, "owned-job", &[], 1).unwrap();
    queue::mark_running(path, "owned-job", "turn-one", 1).unwrap();
    delivery::stage_queue_completion(path, "owned-job", "Final\ncompleted", 10.0).unwrap();
}

pub fn settled(path: &Path) {
    final_pending(path);
    confirm_delivery(path);
}

pub fn settled_for(path: &Path, target: &str, project: &str, parent: i64) {
    queued_for(path, target, project, parent);
    finish_turn(path);
    confirm_delivery(path);
}

fn confirm_delivery(path: &Path) {
    let key = serde_json::json!([31, "completion/v1", "owned-job", 0]).to_string();
    assert_eq!(
        delivery_receipt::begin(path, &key, "fixture-hash").unwrap(),
        delivery_receipt::ReceiptState::New
    );
    assert!(delivery_receipt::confirm(path, &key, "123456").unwrap());
    assert!(delivery::complete(path, "owned-job").unwrap());
}
