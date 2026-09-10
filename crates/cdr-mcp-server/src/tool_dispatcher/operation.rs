use cdr_remote_protocol::output::CoreOutput;
use cdr_remote_protocol::{ProjectOperation, ProjectOperationOutput};
use serde_json::{Map, Value};

use crate::mcp_http::DispatchOutput;

pub(super) fn parse(
    public_name: &str,
    mut arguments: Map<String, Value>,
) -> Result<ProjectOperation, String> {
    let kind = operation_kind(public_name)
        .ok_or_else(|| format!("unknown local project tool: {public_name}"))?;
    arguments.insert("kind".into(), Value::String(kind));
    serde_json::from_value(Value::Object(arguments))
        .map_err(|error| format!("invalid tool arguments: {error}"))
}

pub(super) fn dispatch_output(output: ProjectOperationOutput) -> Result<DispatchOutput, String> {
    if let ProjectOperationOutput::Core(CoreOutput::ImageRetrieve { image, data_base64 }) = output {
        return Ok(DispatchOutput::Image {
            data: data_base64,
            mime_type: image.media_type,
        });
    }
    serde_json::to_value(output)
        .map(DispatchOutput::Structured)
        .map_err(|error| format!("failed to serialize tool output: {error}"))
}

fn operation_kind(name: &str) -> Option<String> {
    Some(match name {
        "repo_diff_summary" | "show_changes" => "repo_diff".into(),
        "list_computer_windows" => "computer_list_windows".into(),
        "activate_computer_window" => "computer_activate".into(),
        "launch_computer_app" => "computer_launch".into(),
        "screenshot_computer_window" => "computer_screenshot".into(),
        "click_computer_window" => "computer_click".into(),
        "drag_computer_window" => "computer_drag".into(),
        "scroll_computer_window" => "computer_scroll".into(),
        "type_computer_text" => "computer_type_text".into(),
        "press_computer_keys" => "computer_press_keys".into(),
        "close_computer_window" => "computer_close".into(),
        "set_computer_clipboard" => "computer_set_clipboard".into(),
        "stop_computer_control" => "computer_stop".into(),
        "checkpoint_list"
        | "checkpoint_restore"
        | "checkpoint_show"
        | "code_search"
        | "command_list"
        | "command_run"
        | "file_apply_patch"
        | "file_create"
        | "git_commit"
        | "git_push"
        | "list_images"
        | "project_rules"
        | "project_status"
        | "repo_status"
        | "retrieve_image"
        | "save_image"
        | "save_image_from_url"
        | "terminal_exec"
        | "terminal_window_activate"
        | "terminal_window_capture"
        | "terminal_window_close"
        | "terminal_window_interrupt"
        | "terminal_window_keys"
        | "terminal_window_list"
        | "terminal_window_open"
        | "terminal_window_type" => name.into(),
        _ => return None,
    })
}
