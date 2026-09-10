use cdr_app_server::{build_approval_response, build_input_response};
use serde_json::json;

#[test]
fn approval_responses_match_python_contract() {
    let cases = [
        (
            "item/commandExecution/requestApproval",
            json!({}),
            "2",
            json!({"decision": "acceptForSession"}),
            "acceptForSession",
        ),
        (
            "item/permissions/requestApproval",
            json!({}),
            "1",
            json!({"permissions": {"network": null, "fileSystem": null}, "scope": "turn"}),
            "accept",
        ),
        (
            "mcpServer/elicitation/request",
            json!({"mode": "url"}),
            "1",
            json!({"action": "accept", "content": {}, "_meta": null}),
            "accept",
        ),
        (
            "execCommandApproval",
            json!({}),
            "3",
            json!({"decision": "denied"}),
            "denied",
        ),
    ];
    for (method, params, answer, expected_payload, expected_action) in cases {
        let (payload, action) =
            build_approval_response(method, &params, answer).expect("approval response");
        assert_eq!(payload, expected_payload);
        assert_eq!(action, expected_action);
    }
    assert!(build_approval_response("unknown/requestApproval", &json!({}), "1").is_err());
}

#[test]
fn input_responses_match_python_contract() {
    let single = build_input_response(
        &json!({"questions": [{"id": "mode", "options": [{"label": "Fast"}, {"label": "Careful"}]}]}),
        "2",
    )
    .expect("single input response");
    assert_eq!(single.answers_by_question["mode"], ["Careful"]);
    assert_eq!(
        single.payload,
        json!({"answers": {"mode": {"answers": ["Careful"]}}})
    );

    let multiple = build_input_response(
        &json!({"questions": [
            {"id": "mode", "options": [{"label": "Fast"}, {"label": "Careful"}]},
            {"id": "confirm", "options": []}
        ]}),
        "mode=Careful;confirm=yes",
    )
    .expect("multi input response");
    assert_eq!(
        multiple.payload,
        json!({"answers": {
            "mode": {"answers": ["Careful"]}, "confirm": {"answers": ["yes"]}
        }})
    );

    assert!(build_input_response(&json!({"questions": [{"id": "mode"}]}), " ").is_err());
    assert!(
        build_input_response(&json!({"questions": [{"id": "mode"}]}), "mode=x;extra=y").is_err()
    );
}
