use cdr_remote_protocol::ProjectOperation;
use cdr_remote_protocol::output::ALL_PROJECT_OUTPUT_KINDS;
use cdr_remote_protocol::request::ALL_PROJECT_OPERATION_KINDS;
use cdr_remote_protocol::{parse_bridge_message, parse_gateway_message};
use serde_json::Value;

#[test]
fn accepts_protocol_ten_hello_and_rejects_nine() {
    parse_bridge_message(r#"{"type":"hello","protocol_version":10,"device_id":"device-a"}"#)
        .expect("protocol 10 hello");
    let error =
        parse_bridge_message(r#"{"type":"hello","protocol_version":9,"device_id":"device-a"}"#)
            .expect_err("protocol 9 must be rejected");
    assert!(
        error.to_string().contains("expected 10"),
        "unexpected error: {error}"
    );
}

#[test]
fn parses_nested_gateway_command_and_validates_operation() {
    let valid = r#"{
        "type":"project_operation",
        "request_id":"request-a",
        "thread_id":"thread-a",
        "deadline_at":"2026-08-31T12:35:56Z",
        "computer_session_id":null,
        "operation":{"kind":"terminal_exec","command":"echo wired"}
    }"#;
    parse_gateway_message(valid).expect("valid gateway command");

    let invalid = valid.replace("echo wired", "");
    assert!(parse_gateway_message(&invalid).is_err());
}

#[test]
fn rejects_unknown_fields_at_the_boundary() {
    let raw = r#"{"type":"hello","protocol_version":10,"device_id":"device-a","extra":true}"#;
    assert!(parse_bridge_message(raw).is_err());
}

#[test]
fn request_deadlines_match_the_python_contract() {
    let cases = [
        (r#"{"kind":"file_create","path":"a","content":"x"}"#, 60),
        (r#"{"kind":"repo_status"}"#, 135),
        (r#"{"kind":"git_push"}"#, 315),
        (
            r#"{"kind":"command_run","command_id":"qa","timeout_seconds":300}"#,
            315,
        ),
        (
            r#"{"kind":"terminal_exec","command":"qa","timeout_seconds":3600}"#,
            3_615,
        ),
    ];
    for (raw, expected) in cases {
        let operation: ProjectOperation = serde_json::from_str(raw).expect("operation JSON");
        assert_eq!(operation.request_lifetime_seconds(), expected);
    }
}

#[test]
fn python_frozen_protocol_fixture_parses_and_round_trips() {
    let raw = include_str!("../../../fixtures/parity/remote_protocol_v10.json");
    let fixture: Value = serde_json::from_str(raw).expect("valid fixture JSON");

    assert_eq!(
        fixture["project_operation_kinds"],
        serde_json::to_value(ALL_PROJECT_OPERATION_KINDS).expect("request kinds")
    );
    assert_eq!(
        fixture["project_output_kinds"],
        serde_json::to_value(ALL_PROJECT_OUTPUT_KINDS).expect("output kinds")
    );

    for value in fixture["bridge_inbound"]
        .as_array()
        .expect("bridge fixtures")
    {
        let encoded = serde_json::to_string(value).expect("fixture JSON");
        let parsed = parse_bridge_message(&encoded).expect("Python bridge fixture");
        let round_trip = serde_json::to_value(parsed).expect("Rust bridge serialization");
        assert_eq!(&round_trip, value);
    }

    for value in fixture["gateway_inbound"]
        .as_array()
        .expect("gateway fixtures")
    {
        let encoded = serde_json::to_string(value).expect("fixture JSON");
        let parsed = parse_gateway_message(&encoded).expect("Python gateway fixture");
        let round_trip = serde_json::to_value(parsed).expect("Rust gateway serialization");
        assert_eq!(&round_trip, value);
    }

    for case in fixture["invalid"].as_array().expect("invalid fixtures") {
        let encoded = serde_json::to_string(&case["message"]).expect("fixture JSON");
        let result = match case["direction"].as_str().expect("direction") {
            "bridge" => parse_bridge_message(&encoded).map(|_| ()),
            "gateway" => parse_gateway_message(&encoded).map(|_| ()),
            other => panic!("unknown direction: {other}"),
        };
        assert!(result.is_err(), "invalid fixture was accepted: {encoded}");
    }
}
