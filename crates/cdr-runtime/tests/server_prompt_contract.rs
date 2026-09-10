use cdr_app_server::{RequestId, ServerRequest, ServerRequestOccurrence};
use cdr_discord::components::{ComponentId, parse_component_id};
use cdr_runtime::server_prompt::{ServerPromptError, build_server_prompt};
use serde_json::json;

fn request(method: &str, params: serde_json::Value) -> ServerRequest {
    request_with_id(RequestId::Integer(7), method, params)
}

fn request_with_id(id: RequestId, method: &str, params: serde_json::Value) -> ServerRequest {
    ServerRequest {
        id,
        occurrence: occurrence(0x11),
        method: method.into(),
        params,
    }
}

#[test]
fn approval_requests_render_four_persistent_buttons() {
    let prompt = build_server_prompt(
        &request(
            "item/commandExecution/requestApproval",
            json!({"threadId":"thread-a", "reason":"Run the requested check"}),
        ),
        1,
    )
    .unwrap();
    assert!(prompt.text.contains("Approval required"));
    assert!(prompt.text.contains("Run the requested check"));
    let row = serde_json::to_value(&prompt.components[0]).unwrap();
    assert_eq!(row["components"].as_array().unwrap().len(), 4);
    assert_eq!(
        row["components"][0]["custom_id"],
        "codex_approval:v2:60e9aec437d0f0f6:a9ea4a6523caf6d7ac871ed0d5496a87:1"
    );
}

#[test]
fn one_question_input_renders_up_to_five_numbered_options() {
    let prompt = build_server_prompt(
        &request(
            "item/tool/requestUserInput",
            json!({"threadId":"thread-a", "questions":[{
                "id":"choice", "header":"Mode", "question":"Which mode?", "options":[
                    {"label":"Safe", "description":"Use the stable path"},
                    {"label":"Fast", "description":"Use the quick path"}
                ]
            }]}),
        ),
        1,
    )
    .unwrap();
    assert!(prompt.text.contains("Which mode?"));
    assert!(prompt.text.contains("1. Safe"));
    let row = serde_json::to_value(&prompt.components[0]).unwrap();
    assert_eq!(
        row["components"][1]["custom_id"],
        "codex_input:v2:60e9aec437d0f0f6:a9ea4a6523caf6d7ac871ed0d5496a87:2"
    );
}

#[test]
fn multi_question_input_explains_the_exact_reply_format() {
    let prompt = build_server_prompt(
        &request(
            "item/tool/requestUserInput",
            json!({"threadId":"thread-a", "questions":[
                {"id":"mode", "question":"Which mode?", "options":[{"label":"Safe"}]},
                {"id":"scope", "question":"Which scope?", "options":[{"label":"All"}]}
            ]}),
        ),
        1,
    )
    .unwrap();
    assert!(prompt.components.is_empty());
    assert!(prompt.text.contains("mode=1; scope=1"));
    assert!(prompt.text.contains("Use | for multiple selections"));
}

#[test]
fn unsupported_or_malformed_requests_fail_closed() {
    assert!(matches!(
        build_server_prompt(&request("unknown", json!({"threadId":"thread-a"})), 1),
        Err(ServerPromptError::Unsupported(_))
    ));
    assert!(build_server_prompt(&request("item/tool/requestUserInput", json!({})), 1).is_err());
}

#[test]
fn nullable_options_render_free_text_without_buttons() {
    let prompt = build_server_prompt(&request("item/tool/requestUserInput", json!({
        "threadId":"thread-a","questions":[{"id":"q","question":"설명해주세요", "options":null}]
    })), 1).unwrap();
    assert!(prompt.components.is_empty());
    assert!(prompt.text.contains("free text"));
}

#[test]
fn secret_input_is_not_invited_into_a_discord_message() {
    let error = build_server_prompt(&request("item/tool/requestUserInput", json!({
        "threadId":"thread-a","questions":[{"id":"q","question":"Private input", "isSecret":true,"options":null}]
    })), 1).unwrap_err();
    assert!(error.to_string().contains("Codex app"));
}

#[test]
fn every_option_is_visible_even_when_only_five_fit_in_buttons() {
    let options: Vec<_> = (1..=8)
        .map(|i| json!({"label":format!("option-{i}"),"description":format!("detail-{i}")}))
        .collect();
    let prompt = build_server_prompt(&request("item/tool/requestUserInput", json!({
        "threadId":"thread-a","questions":[{"id":"q","question":"Choose", "options":options}]
    })), 1).unwrap();
    assert!(prompt.text.contains("8. option-8 — detail-8"));
    let row = serde_json::to_value(&prompt.components[0]).unwrap();
    assert_eq!(row["components"].as_array().unwrap().len(), 5);
}

#[test]
fn malformed_option_is_not_silently_skipped_or_assigned_another_description() {
    assert!(build_server_prompt(&request("item/tool/requestUserInput", json!({
        "threadId":"thread-a","questions":[{"id":"q","question":"Choose", "options":[
            {"label":"", "description":"wrong"},{"label":"valid", "description":"correct"}
        ]}]
    })), 1).is_err());
}

#[test]
fn generation_and_long_typed_request_ids_are_bound_without_growing_the_wire_id() {
    let request = request_with_id(
        RequestId::String("request".repeat(1_000)),
        "item/commandExecution/requestApproval",
        json!({"threadId":"thread".repeat(1_000)}),
    );
    let first = build_server_prompt(&request, 1).unwrap();
    let second = build_server_prompt(&request, 2).unwrap();
    let first_id = custom_id(&first.components[0], 0);
    let second_id = custom_id(&second.components[0], 0);
    assert!(first_id.len() <= 100);
    assert_ne!(first_id, second_id);
    assert!(matches!(
        parse_component_id(first_id),
        Some(ComponentId::BoundApproval { .. })
    ));
}

#[test]
fn a_new_occurrence_changes_an_otherwise_identical_prompt_binding() {
    let mut replacement = request(
        "item/commandExecution/requestApproval",
        json!({"threadId":"thread-a"}),
    );
    let original = build_server_prompt(&replacement, 1).unwrap();
    replacement.occurrence = occurrence(0x22);
    let replacement = build_server_prompt(&replacement, 1).unwrap();

    assert_ne!(
        custom_id(&original.components[0], 0),
        custom_id(&replacement.components[0], 0)
    );
    assert!(custom_id(&replacement.components[0], 0).len() <= 100);
}

const fn occurrence(byte: u8) -> ServerRequestOccurrence {
    ServerRequestOccurrence::from_bytes([byte; 16])
}

fn custom_id(component: &twilight_model::channel::message::Component, index: usize) -> &str {
    let twilight_model::channel::message::Component::ActionRow(row) = component else {
        panic!("expected action row");
    };
    let twilight_model::channel::message::Component::Button(button) = &row.components[index] else {
        panic!("expected button");
    };
    button.custom_id.as_deref().unwrap()
}
