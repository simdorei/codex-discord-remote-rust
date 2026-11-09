use std::io::{Cursor, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use cdr_remote_protocol::output::{
    ComputerOutput, ComputerScreenshotMediaType, ComputerWindowEntry,
};
use cdr_remote_protocol::request::{ComputerApp, ComputerMouseButton, ComputerRequest};
use serde_json::{Value, json};

use super::{CURRENT_PROTOCOL, MAX_INPUT_BYTES, serve};
use crate::computer::{
    ComputerCapture, ComputerController, ComputerError, ComputerIdentity, ComputerPlatform,
};

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jhN8AAAAASUVORK5CYII=";

fn rpc(id: usize, method: &str, params: Value) -> Value {
    let mut request = json!({"jsonrpc":"2.0", "id":id, "method":method});
    request["params"] = params;
    request
}

fn initialize(version: &str) -> Value {
    rpc(
        1,
        "initialize",
        json!({"protocolVersion":version, "capabilities":{},
        "clientInfo":{"name":"test", "version":"1"}}),
    )
}

fn ready() -> Value {
    json!({"jsonrpc":"2.0", "method":"notifications/initialized"})
}

fn call(id: usize, name: &str, arguments: Value) -> Value {
    let mut params = json!({"name":name});
    params["arguments"] = arguments;
    rpc(id, "tools/call", params)
}

fn lines(messages: &[Value]) -> Vec<u8> {
    let mut buffer = Vec::new();
    for message in messages {
        writeln!(buffer, "{message}").unwrap();
    }
    buffer
}

fn run(
    input: &[u8],
    execute: impl FnMut(&ComputerRequest) -> Result<ComputerOutput, String>,
) -> (Result<(), String>, Vec<Value>) {
    let mut output = Vec::new();
    let result = serve(Cursor::new(input), &mut output, execute);
    let messages = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    (result, messages)
}

fn listed() -> ComputerOutput {
    ComputerOutput::ComputerWindows {
        windows: Vec::new(),
    }
}

#[test]
fn tools_need_both_initialization_steps_and_notifications_cannot_execute() {
    let mut executions = 0;
    let input = lines(&[
        ready(),
        call(2, "computer_list_windows", json!({})),
        initialize(CURRENT_PROTOCOL),
        call(3, "computer_list_windows", json!({})),
        ready(),
        json!({"jsonrpc":"2.0", "method":"tools/call", "params":{"name":"computer_launch_app", "arguments":{"app":"chrome"}}}),
        call(4, "computer_list_windows", json!({})),
    ]);
    let (result, replies) = run(&input, |_| {
        executions += 1;
        Ok(listed())
    });
    assert!(result.is_ok());
    assert_eq!(executions, 1);
    assert_eq!(replies.len(), 4);
    assert_eq!(replies[0]["error"]["code"], -32002);
    assert_eq!(replies[2]["error"]["code"], -32002);
    assert_eq!(replies[3]["result"]["isError"], false);
}

#[test]
fn malformed_envelopes_unknown_tools_and_unknown_fields_never_reach_the_engine() {
    let mut input = b"not json\n[]\n".to_vec();
    input.extend(lines(&[
        initialize(CURRENT_PROTOCOL), ready(),
        json!({"jsonrpc":"2.0", "id":null, "method":"tools/call", "params":{"name":"computer_stop"}}),
        json!({"jsonrpc":"2.0", "id":2, "method":"tools/call", "unexpected":true}),
        rpc(3, "tools/call", json!([])),
        call(4, "unknown", json!({})),
        rpc(5, "tools/call", json!({"name":"computer_stop", "extra":1})),
        call(6, "computer_stop", json!({"kind":"computer_launch", "app":"chrome"})),
        call(7, "computer_list_windows", json!({"window_id":1})),
        rpc(8, "tools/list", json!({"cursor":"unknown"})),
    ]));
    let (result, replies) = run(&input, |_| panic!("invalid input reached the engine"));
    assert!(result.is_ok());
    assert_eq!(replies[0]["error"]["code"], -32700);
    assert_eq!(replies[1]["error"]["code"], -32600);
    assert!(
        replies
            .iter()
            .enumerate()
            .all(|(index, reply)| index == 2 || reply.get("error").is_some())
    );
}

#[test]
fn computer_validation_precedes_native_execution_and_counts_unicode_characters() {
    let observed = json!({"window_id":1, "observation_id":"abcdefgh"});
    let invalid = [
        ("computer_activate_window", json!({"window_id":0})),
        ("computer_launch_app", json!({"app":"powershell"})),
        (
            "computer_click",
            json!({"window_id":1,"observation_id":"abcdefgh","x":100_001,"y":0}),
        ),
        (
            "computer_click",
            json!({"window_id":1,"observation_id":"abcdefgh","x":0,"y":0,"click_count":4}),
        ),
        ("computer_drag", observed.clone()),
        (
            "computer_scroll",
            json!({"window_id":1,"observation_id":"abcdefgh","x":0,"y":0}),
        ),
        (
            "computer_scroll",
            json!({"window_id":1,"observation_id":"abcdefgh","x":0,"y":0,"delta_y":10001}),
        ),
        (
            "computer_type_text",
            json!({"window_id":1,"observation_id":"abcdefgh","text":""}),
        ),
        (
            "computer_type_text",
            json!({"window_id":1,"observation_id":"abcdefgh","text":"가".repeat(4097)}),
        ),
        (
            "computer_press_keys",
            json!({"window_id":1,"observation_id":"abcdefgh","keys":[]}),
        ),
        (
            "computer_close_window",
            json!({"window_id":1,"observation_id":"short"}),
        ),
    ];
    let mut messages = vec![initialize(CURRENT_PROTOCOL), ready()];
    for (index, (name, arguments)) in invalid.iter().enumerate() {
        messages.push(call(index + 2, name, arguments.clone()));
    }
    messages.push(call(
        100,
        "computer_type_text",
        json!({"window_id":1,
        "observation_id":"abcdefgh", "text":"가".repeat(4096)}),
    ));
    let mut executions = 0;
    let (result, replies) = run(&lines(&messages), |request| {
        executions += 1;
        let ComputerRequest::ComputerTypeText { text, .. } = request else {
            panic!("wrong request")
        };
        assert_eq!(text.chars().count(), 4096);
        Ok(listed())
    });
    assert!(result.is_ok());
    assert_eq!(executions, 1);
    assert!(
        replies[1..=invalid.len()]
            .iter()
            .all(|reply| reply["error"]["code"] == -32602)
    );
}

#[test]
fn protocol_versions_and_catalog_are_negotiated_without_creating_a_controller() {
    for version in ["2024-11-05", "2025-03-26", CURRENT_PROTOCOL, "2099-01-01"] {
        let (result, replies) = run(
            &lines(&[
                initialize(version),
                ready(),
                rpc(2, "tools/list", json!({})),
                rpc(3, "ping", json!({})),
            ]),
            |_| panic!("protocol message reached the engine"),
        );
        assert!(result.is_ok());
        assert_eq!(
            replies[0]["result"]["protocolVersion"],
            if version == "2099-01-01" {
                CURRENT_PROTOCOL
            } else {
                version
            }
        );
        let tools = replies[1]["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 11);
        assert!(
            tools
                .iter()
                .all(|tool| tool["inputSchema"]["additionalProperties"] == false)
        );
        let scroll = tools
            .iter()
            .find(|tool| tool["name"] == "computer_scroll")
            .unwrap();
        assert_eq!(scroll["inputSchema"]["anyOf"].as_array().unwrap().len(), 2);
    }
}

#[test]
fn duplicate_request_ids_do_not_repeat_actions() {
    let mut executions = 0;
    let action = call(2, "computer_launch_app", json!({"app":"notepad"}));
    let (result, replies) = run(
        &lines(&[
            initialize(CURRENT_PROTOCOL),
            ready(),
            action.clone(),
            action,
        ]),
        |_| {
            executions += 1;
            Ok(listed())
        },
    );
    assert!(result.is_ok());
    assert_eq!(executions, 1);
    assert_eq!(replies[2]["error"]["code"], -32600);
}

#[test]
fn oversized_input_closes_before_any_trailing_action() {
    let mut input = vec![b' '; MAX_INPUT_BYTES + 1];
    input.extend(lines(&[
        initialize(CURRENT_PROTOCOL),
        ready(),
        call(2, "computer_stop", json!({})),
    ]));
    let (result, replies) = run(&input, |_| panic!("oversized frame executed an action"));
    assert!(result.is_err());
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["error"]["code"], -32600);
    let mut exact = vec![b' '; MAX_INPUT_BYTES - 3];
    exact.extend_from_slice(b"{}\n");
    assert!(run(&exact, |_| unreachable!()).0.is_ok());
}

#[test]
fn eof_without_a_frame_terminator_never_executes_the_partial_request() {
    let mut input = lines(&[initialize(CURRENT_PROTOCOL), ready()]);
    input.extend(
        call(2, "computer_launch_app", json!({"app":"chrome"}))
            .to_string()
            .as_bytes(),
    );
    let (result, replies) = run(&input, |_| panic!("incomplete frame reached the engine"));
    assert!(result.is_err());
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[1]["error"]["code"], -32700);
}

#[test]
fn screenshot_png_is_sent_once_and_metadata_excludes_base64() {
    for version in [CURRENT_PROTOCOL, "2024-11-05"] {
        let (result, replies) = run(
            &lines(&[
                initialize(version),
                ready(),
                call(2, "computer_screenshot", json!({"window_id":7})),
            ]),
            |_| {
                Ok(ComputerOutput::ComputerScreenshot {
                    observation_id: "abcdefgh".into(),
                    window: window(),
                    media_type: ComputerScreenshotMediaType::Png,
                    data_base64: PNG.into(),
                })
            },
        );
        assert!(result.is_ok());
        let result = &replies[1]["result"];
        assert_eq!(result["content"][1]["type"], "image");
        assert_eq!(result["content"][1]["mimeType"], "image/png");
        assert_eq!(result["content"][1]["data"], PNG);
        let metadata: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(metadata.get("data_base64").is_none());
        assert_eq!(result.to_string().matches(PNG).count(), 1);
        assert_eq!(
            result.get("structuredContent").is_some(),
            version == CURRENT_PROTOCOL
        );
    }
}

#[test]
fn input_loop_preserves_engine_observation_consumption_and_permanent_stop() {
    let platform = Arc::new(FakePlatform::default());
    let controller =
        ComputerController::with_platform_and_ttl(platform.clone(), Duration::from_secs(30));
    let ComputerOutput::ComputerScreenshot { observation_id, .. } = controller
        .execute(&ComputerRequest::ComputerScreenshot { window_id: 7 })
        .unwrap()
    else {
        panic!("missing screenshot")
    };
    let arguments = json!({"window_id":7, "observation_id":observation_id, "x":0, "y":0});
    let (result, replies) = run(
        &lines(&[
            initialize(CURRENT_PROTOCOL),
            ready(),
            call(2, "computer_click", arguments.clone()),
            call(3, "computer_click", arguments),
            call(4, "computer_stop", json!({})),
            call(5, "computer_list_windows", json!({})),
            call(6, "computer_stop", json!({})),
        ]),
        |request| {
            controller
                .execute(request)
                .map_err(|error| error.to_string())
        },
    );
    assert!(result.is_ok());
    assert_eq!(replies[1]["result"]["isError"], false);
    assert_eq!(replies[2]["result"]["isError"], true);
    assert!(
        replies[2]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("fresh screenshot")
    );
    assert_eq!(replies[4]["result"]["isError"], true);
    assert_eq!(platform.clicks.load(Ordering::Relaxed), 1);
    assert_eq!(platform.stops.load(Ordering::Relaxed), 1);
}

fn window() -> ComputerWindowEntry {
    ComputerWindowEntry {
        window_id: 7,
        title: "fixture".into(),
        process_name: "fixture".into(),
        left: 0,
        top: 0,
        width: 1,
        height: 1,
        active: true,
    }
}

#[derive(Default)]
struct FakePlatform {
    clicks: AtomicUsize,
    stops: AtomicUsize,
}

impl ComputerPlatform for FakePlatform {
    fn list_windows(&self) -> Result<Vec<ComputerWindowEntry>, ComputerError> {
        Ok(vec![window()])
    }
    fn screenshot(&self, window_id: u64) -> Result<ComputerCapture, ComputerError> {
        Ok(ComputerCapture {
            window: window(),
            png: STANDARD.decode(PNG).unwrap(),
            identity: ComputerIdentity {
                window_id,
                process_id: 1,
                process_path: "fixture.exe".into(),
                title_digest: "fixture".into(),
                left: 0,
                top: 0,
                width: 1,
                height: 1,
            },
        })
    }
    fn activate(&self, _: u64) -> Result<ComputerWindowEntry, ComputerError> {
        Ok(window())
    }
    fn launch(&self, _: ComputerApp) -> Result<(), ComputerError> {
        Ok(())
    }
    fn click(
        &self,
        _: &ComputerIdentity,
        _: u32,
        _: u32,
        _: ComputerMouseButton,
        _: u8,
    ) -> Result<(), ComputerError> {
        self.clicks.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn drag(
        &self,
        _: &ComputerIdentity,
        _: (u32, u32),
        _: (u32, u32),
    ) -> Result<(), ComputerError> {
        Ok(())
    }
    fn scroll(
        &self,
        _: &ComputerIdentity,
        _: (u32, u32),
        _: (i32, i32),
    ) -> Result<(), ComputerError> {
        Ok(())
    }
    fn type_text(&self, _: &ComputerIdentity, _: &str) -> Result<(), ComputerError> {
        Ok(())
    }
    fn press_keys(&self, _: &ComputerIdentity, _: &[String]) -> Result<(), ComputerError> {
        Ok(())
    }
    fn close(&self, _: &ComputerIdentity) -> Result<(), ComputerError> {
        Ok(())
    }
    fn set_clipboard(&self, _: &ComputerIdentity, _: &str) -> Result<(), ComputerError> {
        Ok(())
    }
    fn stop(&self) -> Result<(), ComputerError> {
        self.stops.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
