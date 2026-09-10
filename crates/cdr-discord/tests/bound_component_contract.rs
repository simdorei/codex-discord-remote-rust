use cdr_discord::components::{
    ApprovalAnswer, ComponentId, ComponentRequestId, bound_approval_button_row,
    bound_input_button_row, parse_component_id, persistent_claim_key, request_fingerprint,
    thread_fingerprint,
};
use twilight_model::channel::message::Component;

const THREAD_A_FP: &str = "60e9aec437d0f0f6";
const INTEGER_7_FP: &str = "a9ea4a6523caf6d7ac871ed0d5496a87";
const GENERATION: u64 = 1;
const OCCURRENCE_A: [u8; 16] = [0x11; 16];
const OCCURRENCE_B: [u8; 16] = [0x22; 16];

#[test]
fn occurrence_separates_reused_request_ids_without_growing_the_wire_id() {
    let first = request_fingerprint(GENERATION, &OCCURRENCE_A, ComponentRequestId::Integer(7));
    let second = request_fingerprint(GENERATION, &OCCURRENCE_B, ComponentRequestId::Integer(7));
    assert_ne!(first, second);

    let ids = custom_ids(
        bound_approval_button_row(
            "thread-a",
            GENERATION,
            &OCCURRENCE_A,
            ComponentRequestId::Integer(7),
        )
        .unwrap(),
    );
    assert!(ids.iter().all(|custom_id| custom_id.len() <= 100));
}

#[test]
fn bound_rows_emit_strict_parseable_v2_ids() {
    let approval = custom_ids(
        bound_approval_button_row(
            "thread-a",
            GENERATION,
            &OCCURRENCE_A,
            ComponentRequestId::Integer(7),
        )
        .unwrap(),
    );
    assert_eq!(
        approval[0],
        format!("codex_approval:v2:{THREAD_A_FP}:{INTEGER_7_FP}:1")
    );
    assert_eq!(
        parse_component_id(&approval[1]),
        Some(ComponentId::BoundApproval {
            thread_fingerprint: THREAD_A_FP.into(),
            request_fingerprint: INTEGER_7_FP.into(),
            answer: ApprovalAnswer::ApproveSession,
        })
    );

    let options = vec![("2".to_owned(), "Fast".to_owned())];
    let input = custom_ids(
        bound_input_button_row(
            "thread-a",
            GENERATION,
            &OCCURRENCE_A,
            ComponentRequestId::Integer(7),
            &options,
        )
        .unwrap(),
    );
    assert_eq!(
        input[0],
        format!("codex_input:v2:{THREAD_A_FP}:{INTEGER_7_FP}:2")
    );
    assert_eq!(
        parse_component_id(&input[0]),
        Some(ComponentId::BoundInput {
            thread_fingerprint: THREAD_A_FP.into(),
            request_fingerprint: INTEGER_7_FP.into(),
            value: "2".into(),
        })
    );
}

#[test]
fn hashes_bound_long_ids_and_separates_typed_request_ids() {
    let long_thread = "thread".repeat(1_000);
    let long_request = "request".repeat(1_000);
    let ids = custom_ids(
        bound_approval_button_row(
            &long_thread,
            GENERATION,
            &OCCURRENCE_A,
            ComponentRequestId::String(&long_request),
        )
        .unwrap(),
    );
    assert!(ids.iter().all(|custom_id| custom_id.len() <= 100));
    assert_eq!(thread_fingerprint("thread-a").unwrap(), THREAD_A_FP);
    assert_eq!(
        request_fingerprint(GENERATION, &OCCURRENCE_A, ComponentRequestId::String("42")),
        "5e6e7f5c227ef203a644f42457b5127e"
    );
    assert_eq!(
        request_fingerprint(GENERATION, &OCCURRENCE_A, ComponentRequestId::Integer(42)),
        "573643854409d20702cda663174076ab"
    );
    assert_ne!(
        request_fingerprint(GENERATION, &OCCURRENCE_A, ComponentRequestId::String("42")),
        request_fingerprint(GENERATION, &OCCURRENCE_A, ComponentRequestId::Integer(42))
    );
    assert_ne!(
        request_fingerprint(GENERATION, &OCCURRENCE_A, ComponentRequestId::Integer(42)),
        request_fingerprint(
            GENERATION + 1,
            &OCCURRENCE_A,
            ComponentRequestId::Integer(42)
        )
    );
    assert_ne!(
        thread_fingerprint("thread-a").unwrap(),
        thread_fingerprint("thread-b").unwrap()
    );
}

#[test]
fn v2_parser_rejects_noncanonical_fingerprints_and_unsafe_values() {
    assert!(
        parse_component_id("codex_approval:v2:60E9AEC437D0F0F6:3abc59e3fff33ba110f5603cb0807ed3:1")
            .is_none()
    );
    assert!(
        parse_component_id("codex_approval:v2:60e9aec437d0f0f:3abc59e3fff33ba110f5603cb0807ed3:1")
            .is_none()
    );
    assert!(
        parse_component_id(
            "codex_input:v2:60e9aec437d0f0f6:3abc59e3fff33ba110f5603cb0807ed:unsafe value"
        )
        .is_none()
    );
    assert!(
        parse_component_id(
            "codex_approval:v2:60e9aec437d0f0f6:3abc59e3fff33ba110f5603cb0807ed3: 1"
        )
        .is_none()
    );
}

#[test]
fn bound_claims_include_the_request_but_not_the_selected_answer() {
    let first = format!("codex_approval:v2:{THREAD_A_FP}:{INTEGER_7_FP}:1");
    let different_answer = format!("codex_approval:v2:{THREAD_A_FP}:{INTEGER_7_FP}:3");
    let different_request =
        format!("codex_approval:v2:{THREAD_A_FP}:573643854409d20702cda663174076ab:1");
    assert_eq!(
        persistent_claim_key(91, &first),
        persistent_claim_key(91, &different_answer)
    );
    assert_ne!(
        persistent_claim_key(91, &first),
        persistent_claim_key(91, &different_request)
    );
    assert_ne!(
        persistent_claim_key(91, &first),
        persistent_claim_key(92, &first)
    );

    let input = format!("codex_input:v2:{THREAD_A_FP}:{INTEGER_7_FP}:1");
    let input_answer = format!("codex_input:v2:{THREAD_A_FP}:{INTEGER_7_FP}:2");
    let input_request = format!("codex_input:v2:{THREAD_A_FP}:573643854409d20702cda663174076ab:1");
    assert_eq!(
        persistent_claim_key(91, &input),
        persistent_claim_key(91, &input_answer)
    );
    assert_ne!(
        persistent_claim_key(91, &input),
        persistent_claim_key(91, &input_request)
    );
    assert_ne!(
        persistent_claim_key(91, &first),
        persistent_claim_key(91, &input)
    );
}

fn custom_ids(component: Component) -> Vec<String> {
    let encoded = serde_json::to_value(component).unwrap();
    encoded["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|button| button["custom_id"].as_str().unwrap().to_owned())
        .collect()
}
