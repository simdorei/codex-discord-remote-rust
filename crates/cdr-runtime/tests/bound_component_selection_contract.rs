use cdr_app_server::{RequestId, ServerRequest, ServerRequestOccurrence};
use cdr_discord::components::{ApprovalAnswer, ComponentId, parse_component_id};
use cdr_runtime::component_worker::{ComponentWorkerError, build_component_response};
use cdr_runtime::server_prompt::build_server_prompt;
use serde_json::json;

const GENERATION: u64 = 1;
const THREAD_A_FP: &str = "60e9aec437d0f0f6";
const INTEGER_1_FP: &str = "2bdb820fc6cefaa5d7a9ee7f8050232d";
const PRE_OCCURRENCE_INTEGER_1_FP: &str = "c33941b38986b5fe1f0ae04ac8e5ca26";

#[test]
fn emitted_bound_button_selects_its_original_request_end_to_end() {
    let original = approval(1, "thread-a");
    let prompt = build_server_prompt(&original, GENERATION).unwrap();
    let encoded = serde_json::to_value(&prompt.components[0]).unwrap();
    let custom_id = encoded["components"][0]["custom_id"].as_str().unwrap();
    let component = parse_component_id(custom_id).unwrap();

    let requests = vec![original, approval(2, "thread-a")];
    let response = build_component_response(&component, &requests, GENERATION).unwrap();
    assert_eq!(response.request_id, RequestId::Integer(1));
    assert_eq!(response.occurrence, occurrence(0x11));
}

#[test]
fn bound_approval_selects_its_older_exact_request() {
    let requests = vec![approval(1, "thread-a"), approval(2, "thread-a")];
    let response = build_component_response(
        &ComponentId::BoundApproval {
            thread_fingerprint: THREAD_A_FP.into(),
            request_fingerprint: INTEGER_1_FP.into(),
            answer: ApprovalAnswer::Approve,
        },
        &requests,
        GENERATION,
    )
    .unwrap();

    assert_eq!(response.request_id, RequestId::Integer(1));
    assert_eq!(response.generation, GENERATION);
}

#[test]
fn bound_input_selects_its_older_exact_request() {
    let requests = vec![input(1, "thread-a"), input(2, "thread-a")];
    let response = build_component_response(
        &ComponentId::BoundInput {
            thread_fingerprint: THREAD_A_FP.into(),
            request_fingerprint: INTEGER_1_FP.into(),
            value: "1".into(),
        },
        &requests,
        GENERATION,
    )
    .unwrap();

    assert_eq!(response.request_id, RequestId::Integer(1));
    assert_eq!(response.generation, GENERATION);
    assert_eq!(
        response.payload,
        json!({"answers":{"mode":{"answers":["Safe"]}}})
    );
}

#[test]
fn stale_generation_or_missing_binding_fails_closed() {
    let component = ComponentId::BoundApproval {
        thread_fingerprint: THREAD_A_FP.into(),
        request_fingerprint: INTEGER_1_FP.into(),
        answer: ApprovalAnswer::Approve,
    };
    assert!(matches!(
        build_component_response(&component, &[approval(1, "thread-a")], GENERATION + 1),
        Err(ComponentWorkerError::NoPendingRequest)
    ));
    assert!(matches!(
        build_component_response(&component, &[approval(2, "thread-a")], GENERATION),
        Err(ComponentWorkerError::NoPendingRequest)
    ));
}

#[test]
fn pre_occurrence_or_reused_occurrence_binding_fails_closed() {
    let old_wire_component = ComponentId::BoundApproval {
        thread_fingerprint: THREAD_A_FP.into(),
        request_fingerprint: PRE_OCCURRENCE_INTEGER_1_FP.into(),
        answer: ApprovalAnswer::Approve,
    };
    assert!(matches!(
        build_component_response(&old_wire_component, &[approval(1, "thread-a")], GENERATION),
        Err(ComponentWorkerError::NoPendingRequest)
    ));

    let old_occurrence_component = ComponentId::BoundApproval {
        thread_fingerprint: THREAD_A_FP.into(),
        request_fingerprint: INTEGER_1_FP.into(),
        answer: ApprovalAnswer::Approve,
    };
    let replacement = approval_with_occurrence(1, "thread-a", occurrence(0x22));
    assert!(matches!(
        build_component_response(&old_occurrence_component, &[replacement], GENERATION),
        Err(ComponentWorkerError::NoPendingRequest)
    ));
}

#[test]
fn exact_binding_with_the_wrong_method_kind_fails_closed() {
    let component = ComponentId::BoundApproval {
        thread_fingerprint: THREAD_A_FP.into(),
        request_fingerprint: INTEGER_1_FP.into(),
        answer: ApprovalAnswer::Approve,
    };
    assert!(matches!(
        build_component_response(&component, &[input(1, "thread-a")], GENERATION),
        Err(ComponentWorkerError::NoPendingRequest)
    ));
}

#[test]
fn duplicate_or_colliding_bound_identity_is_ambiguous_before_kind_filtering() {
    let component = ComponentId::BoundApproval {
        thread_fingerprint: THREAD_A_FP.into(),
        request_fingerprint: INTEGER_1_FP.into(),
        answer: ApprovalAnswer::Approve,
    };
    for requests in [
        vec![approval(1, "thread-a"), approval(1, "thread-a")],
        vec![approval(1, "thread-a"), input(1, "thread-a")],
    ] {
        assert!(matches!(
            build_component_response(&component, &requests, GENERATION),
            Err(ComponentWorkerError::AmbiguousPendingRequest)
        ));
    }
}

#[test]
fn legacy_components_fail_closed_without_exactly_one_matching_candidate() {
    let approval_component = ComponentId::Approval {
        thread_id: "thread-a".into(),
        answer: ApprovalAnswer::Approve,
    };
    assert!(matches!(
        build_component_response(&approval_component, &[input(1, "thread-a")], GENERATION),
        Err(ComponentWorkerError::NoPendingRequest)
    ));
    assert!(matches!(
        build_component_response(
            &approval_component,
            &[approval(1, "other-thread")],
            GENERATION
        ),
        Err(ComponentWorkerError::NoPendingRequest)
    ));
    assert!(matches!(
        build_component_response(
            &approval_component,
            &[approval(1, "thread-a"), approval(2, "thread-a")],
            GENERATION
        ),
        Err(ComponentWorkerError::AmbiguousPendingRequest)
    ));

    let input_component = ComponentId::Input {
        thread_id: "thread-a".into(),
        value: "1".into(),
    };
    assert!(matches!(
        build_component_response(&input_component, &[approval(1, "thread-a")], GENERATION),
        Err(ComponentWorkerError::NoPendingRequest)
    ));
    assert!(matches!(
        build_component_response(&input_component, &[input(1, "other-thread")], GENERATION),
        Err(ComponentWorkerError::NoPendingRequest)
    ));
    assert!(matches!(
        build_component_response(
            &input_component,
            &[input(1, "thread-a"), input(2, "thread-a")],
            GENERATION
        ),
        Err(ComponentWorkerError::AmbiguousPendingRequest)
    ));
}

fn approval(id: i64, thread_id: &str) -> ServerRequest {
    approval_with_occurrence(id, thread_id, occurrence(0x11))
}

fn approval_with_occurrence(
    id: i64,
    thread_id: &str,
    occurrence: ServerRequestOccurrence,
) -> ServerRequest {
    ServerRequest {
        id: RequestId::Integer(id),
        occurrence,
        method: "item/commandExecution/requestApproval".into(),
        params: json!({"threadId":thread_id}),
    }
}

fn input(id: i64, thread_id: &str) -> ServerRequest {
    ServerRequest {
        id: RequestId::Integer(id),
        occurrence: occurrence(0x11),
        method: "item/tool/requestUserInput".into(),
        params: json!({"threadId":thread_id,"questions":[{
            "id":"mode","options":[{"label":"Safe"},{"label":"Fast"}]
        }]}),
    }
}

const fn occurrence(byte: u8) -> ServerRequestOccurrence {
    ServerRequestOccurrence::from_bytes([byte; 16])
}
