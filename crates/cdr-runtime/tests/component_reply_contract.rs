use cdr_app_server::{RequestId, ServerRequest, ServerRequestOccurrence};
use cdr_discord::components::{
    ApprovalAnswer, BusyAction, ComponentId, ComponentRequestId, request_fingerprint,
};
use cdr_runtime::component_worker::{
    BusyComponentError, ComponentWorkerError, build_component_response, validate_busy_choice,
};
use cdr_store::claims::BusyChoice;
use serde_json::json;

fn request(id: i64, method: &str, params: serde_json::Value) -> ServerRequest {
    request_with_occurrence(id, method, params, occurrence())
}

fn request_with_occurrence(
    id: i64,
    method: &str,
    params: serde_json::Value,
    occurrence: ServerRequestOccurrence,
) -> ServerRequest {
    ServerRequest {
        id: RequestId::Integer(id),
        occurrence,
        method: method.into(),
        params,
    }
}

#[test]
fn legacy_component_preserves_the_unique_candidate_occurrence_and_generation() {
    let requests = vec![
        request(
            1,
            "item/tool/requestUserInput",
            json!({"threadId":"t","questions":[]}),
        ),
        request(
            2,
            "item/commandExecution/requestApproval",
            json!({"threadId":"t"}),
        ),
        request(
            3,
            "item/commandExecution/requestApproval",
            json!({"threadId":"other-thread"}),
        ),
    ];
    let response = build_component_response(
        &ComponentId::Approval {
            thread_id: "t".into(),
            answer: ApprovalAnswer::ApproveSession,
        },
        &requests,
        1,
    )
    .unwrap();
    assert_eq!(response.request_id, RequestId::Integer(2));
    assert_eq!(response.occurrence, occurrence());
    assert_eq!(response.generation, 1);
    assert_eq!(response.payload, json!({"decision":"acceptForSession"}));
}

#[test]
fn legacy_input_selects_only_the_unique_exact_thread_input_candidate() {
    let selected_occurrence = ServerRequestOccurrence::from_bytes([0x22; 16]);
    let requests = vec![
        request_with_occurrence(
            4,
            "item/tool/requestUserInput",
            json!({"threadId":"other-thread","questions":[{
                "id":"mode","options":[{"label":"Other"}]
            }]}),
            ServerRequestOccurrence::from_bytes([0x33; 16]),
        ),
        request(
            5,
            "item/commandExecution/requestApproval",
            json!({"threadId":"t"}),
        ),
        request_with_occurrence(
            6,
            "item/tool/requestUserInput",
            json!({"threadId":"t","questions":[{
                "id":"mode","options":[{"label":"Safe"},{"label":"Fast"}]
            }]}),
            selected_occurrence,
        ),
    ];

    let response = build_component_response(
        &ComponentId::Input {
            thread_id: "t".into(),
            value: "2".into(),
        },
        &requests,
        7,
    )
    .unwrap();

    assert_eq!(response.request_id, RequestId::Integer(6));
    assert_eq!(response.occurrence, selected_occurrence);
    assert_eq!(response.generation, 7);
    assert_eq!(
        response.payload,
        json!({"answers":{"mode":{"answers":["Fast"]}}})
    );
}

#[test]
fn legacy_approval_recognizes_every_supported_approval_method() {
    for (id, method, extra_params) in [
        (10, "item/commandExecution/requestApproval", json!({})),
        (11, "item/fileChange/requestApproval", json!({})),
        (12, "item/permissions/requestApproval", json!({})),
        (13, "execCommandApproval", json!({})),
        (14, "applyPatchApproval", json!({})),
        (15, "mcpServer/elicitation/request", json!({"mode":"url"})),
    ] {
        let mut params = extra_params;
        params["threadId"] = json!("t");
        let response = build_component_response(
            &ComponentId::Approval {
                thread_id: "t".into(),
                answer: ApprovalAnswer::Approve,
            },
            &[request(id, method, params)],
            9,
        )
        .unwrap();
        assert_eq!(response.request_id, RequestId::Integer(id), "{method}");
    }
}

#[test]
fn legacy_method_kind_matching_excludes_near_matches() {
    let approval = ComponentId::Approval {
        thread_id: "t".into(),
        answer: ApprovalAnswer::Approve,
    };
    assert!(matches!(
        build_component_response(
            &approval,
            &[request(
                20,
                "mcpServer/elicitation/request",
                json!({"threadId":"t","mode":"form"}),
            )],
            1,
        ),
        Err(ComponentWorkerError::NoPendingRequest)
    ));

    let input = ComponentId::Input {
        thread_id: "t".into(),
        value: "1".into(),
    };
    assert!(matches!(
        build_component_response(
            &input,
            &[request(
                21,
                "item/tool/requestUserInput/preview",
                json!({"threadId":"t"}),
            )],
            1,
        ),
        Err(ComponentWorkerError::NoPendingRequest)
    ));
}

#[test]
fn input_button_builds_the_exact_question_answer_payload() {
    let requests = vec![request(
        3,
        "item/tool/requestUserInput",
        json!({"threadId":"t", "questions":[{
            "id":"mode", "options":[{"label":"Safe"},{"label":"Fast"}]
        }]}),
    )];
    let response = build_component_response(
        &ComponentId::BoundInput {
            thread_fingerprint: "0060f02d44e3af89".into(),
            request_fingerprint: request_fingerprint(
                1,
                occurrence().as_bytes(),
                ComponentRequestId::Integer(3),
            ),
            value: "2".into(),
        },
        &requests,
        1,
    )
    .unwrap();
    assert_eq!(
        response.payload,
        json!({"answers":{"mode":{"answers":["Fast"]}}})
    );
    assert_eq!(response.occurrence, occurrence());
}

#[test]
fn missing_or_wrong_request_type_is_not_silently_substituted() {
    assert!(matches!(
        build_component_response(
            &ComponentId::BoundApproval {
                thread_fingerprint: "0060f02d44e3af89".into(),
                request_fingerprint: request_fingerprint(
                    1,
                    occurrence().as_bytes(),
                    ComponentRequestId::Integer(3),
                ),
                answer: ApprovalAnswer::Approve,
            },
            &[],
            1,
        ),
        Err(ComponentWorkerError::NoPendingRequest)
    ));
}

const fn occurrence() -> ServerRequestOccurrence {
    ServerRequestOccurrence::from_bytes([0x11; 16])
}

#[test]
fn busy_choice_authorization_checks_user_channel_not_a_stale_activity_snapshot() {
    let choice = BusyChoice {
        choice_id: "0123456789abcdef01234567".into(),
        owner_user_id: 7,
        channel_id: 8,
        target_thread_id: Some("thread-a".into()),
        prompt: "next".into(),
        allow_steer: false,
        created_at: 1.0,
        expires_at: 2.0,
    };
    assert!(validate_busy_choice(&choice, BusyAction::Queue, 7, 8).is_ok());
    assert!(matches!(
        validate_busy_choice(&choice, BusyAction::Queue, 9, 8),
        Err(BusyComponentError::WrongUser)
    ));
    assert!(matches!(
        validate_busy_choice(&choice, BusyAction::Queue, 7, 9),
        Err(BusyComponentError::WrongChannel)
    ));
    assert!(validate_busy_choice(&choice, BusyAction::Steer, 7, 8).is_ok());
}
