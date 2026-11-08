use cdr_core::Validate;
use cdr_remote_protocol::request::ComputerRequest;
use serde_json::{Map, Value, json};

pub(super) const INSTRUCTIONS: &str = "This is the explicitly enabled CDR Windows desktop controller. Select it explicitly for the task; never use it as an automatic fallback for another tool's access denial. Use list_windows to find existing windows, then activate_window and screenshot. All coordinates are relative to that window's screenshot. Each input/close consumes one observation_id; capture again before the next action, and within 30 seconds. launch_app creates temporary owned Chrome guest or Notepad windows; computer_stop and stdin EOF close those owned windows. Use existing windows through list/activate when they must remain open. Stop is permanent for this MCP process. Do not type secrets or one-time codes. The active Windows desktop is shared; avoid simultaneous work by other clients.";

const DEFINITIONS: [(&str, &str, &str); 11] = [
    (
        "computer_list_windows",
        "computer_list_windows",
        "List windows on this Windows desktop.",
    ),
    (
        "computer_activate_window",
        "computer_activate",
        "Bring a listed window forward, then take a fresh screenshot.",
    ),
    (
        "computer_launch_app",
        "computer_launch",
        "Launch temporary owned Chrome guest or Notepad windows. Stop/EOF closes owned windows.",
    ),
    (
        "computer_screenshot",
        "computer_screenshot",
        "Capture a window and return its PNG plus a single-use observation_id valid for 30 seconds.",
    ),
    (
        "computer_click",
        "computer_click",
        "Click screenshot-relative coordinates using a fresh, unconsumed observation_id.",
    ),
    (
        "computer_drag",
        "computer_drag",
        "Drag between screenshot-relative points using a fresh, unconsumed observation_id.",
    ),
    (
        "computer_scroll",
        "computer_scroll",
        "Scroll at a screenshot-relative point; delta_x/delta_y are signed native scroll amounts.",
    ),
    (
        "computer_type_text",
        "computer_type_text",
        "Type text into the observed window. Never enter secrets or one-time codes.",
    ),
    (
        "computer_press_keys",
        "computer_press_keys",
        "Press a chord of one to four Windows key names in the observed window.",
    ),
    (
        "computer_close_window",
        "computer_close",
        "Request closing the observed window. Existing save dialogs remain visible.",
    ),
    (
        "computer_stop",
        "computer_stop",
        "Stop this MCP process's computer control and clean up its temporary owned windows.",
    ),
];

pub(super) fn tools() -> Vec<Value> {
    DEFINITIONS
        .iter()
        .map(|(name, kind, description)| {
            json!({
                "name":name, "description":description, "inputSchema":schema(kind)
            })
        })
        .collect()
}

pub(super) fn request(params: &Map<String, Value>) -> Result<ComputerRequest, String> {
    if !super::params_match(params, &["name", "arguments"]) {
        return Err("Unknown tools/call parameter or invalid _meta.".into());
    }
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or("A tool name is required.")?;
    let (_, kind, _) = DEFINITIONS
        .iter()
        .find(|(tool, _, _)| *tool == name)
        .ok_or("Unknown computer tool.")?;
    let mut arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}))
        .as_object()
        .cloned()
        .ok_or("Tool arguments must be an object.")?;
    let schema = schema(kind);
    if arguments
        .keys()
        .any(|key| schema["properties"].get(key).is_none())
    {
        return Err("Unknown computer tool argument.".into());
    }
    arguments.insert("kind".into(), json!(kind));
    let request: ComputerRequest = serde_json::from_value(Value::Object(arguments))
        .map_err(|error| format!("Invalid computer arguments: {error}"))?;
    request
        .validate()
        .map_err(|error| format!("Invalid computer arguments: {error}"))?;
    Ok(request)
}

fn schema(kind: &str) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    if !matches!(
        kind,
        "computer_list_windows" | "computer_launch" | "computer_stop"
    ) {
        properties.insert(
            "window_id".into(),
            json!({"type":"integer", "minimum":1, "maximum":u64::MAX}),
        );
        required.push("window_id");
    }
    if matches!(
        kind,
        "computer_click"
            | "computer_drag"
            | "computer_scroll"
            | "computer_type_text"
            | "computer_press_keys"
            | "computer_close"
    ) {
        properties.insert(
            "observation_id".into(),
            json!({"type":"string", "minLength":8, "maxLength":100}),
        );
        required.push("observation_id");
    }
    match kind {
        "computer_launch" => {
            properties.insert(
                "app".into(),
                json!({"type":"string", "enum":["chrome", "notepad"]}),
            );
            required.push("app");
        }
        "computer_click" | "computer_scroll" => {
            coordinates(&mut properties, &mut required, &["x", "y"]);
        }
        "computer_drag" => {
            coordinates(
                &mut properties,
                &mut required,
                &["start_x", "start_y", "end_x", "end_y"],
            );
        }
        "computer_type_text" => {
            properties.insert(
                "text".into(),
                json!({"type":"string", "minLength":1, "maxLength":4096}),
            );
            required.push("text");
        }
        "computer_press_keys" => {
            properties.insert(
                "keys".into(),
                json!({"type":"array", "items":{"type":"string"}, "minItems":1, "maxItems":4}),
            );
            required.push("keys");
        }
        _ => {}
    }
    if kind == "computer_click" {
        properties.insert(
            "button".into(),
            json!({"type":"string", "enum":["left", "right", "middle"], "default":"left"}),
        );
        properties.insert(
            "click_count".into(),
            json!({"type":"integer", "minimum":1, "maximum":3, "default":1}),
        );
    }
    if kind == "computer_scroll" {
        for key in ["delta_x", "delta_y"] {
            properties.insert(
                key.into(),
                json!({"type":"integer", "minimum":-10_000, "maximum":10_000, "default":0}),
            );
        }
    }
    let mut schema = json!({"type":"object", "properties":properties,
        "required":required, "additionalProperties":false});
    if kind == "computer_scroll" {
        schema["anyOf"] = json!([
            {"required":["delta_x"], "properties":{"delta_x":{"not":{"const":0}}}},
            {"required":["delta_y"], "properties":{"delta_y":{"not":{"const":0}}}},
        ]);
    }
    schema
}

fn coordinates(
    properties: &mut Map<String, Value>,
    required: &mut Vec<&'static str>,
    names: &[&'static str],
) {
    for name in names {
        properties.insert(
            (*name).into(),
            json!({"type":"integer", "minimum":0, "maximum":100_000}),
        );
        required.push(*name);
    }
}
