use std::collections::BTreeSet;

use cdr_mcp_server::capability::{CAPABILITY_GROUPS, EXPECTED_TOOL_NAMES};
use cdr_remote_protocol::request::ALL_PROJECT_OPERATION_KINDS;

#[test]
fn public_mcp_inventory_has_all_51_unique_python_parity_tools() {
    assert_eq!(EXPECTED_TOOL_NAMES.len(), 51);
    let unique = EXPECTED_TOOL_NAMES.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(unique.len(), EXPECTED_TOOL_NAMES.len());
    let grouped = CAPABILITY_GROUPS
        .iter()
        .flat_map(|group| group.tools.iter().copied())
        .collect::<BTreeSet<_>>();
    assert_eq!(grouped, unique);
    assert_eq!(
        CAPABILITY_GROUPS
            .iter()
            .map(|group| group.tools.len())
            .sum::<usize>(),
        EXPECTED_TOOL_NAMES.len()
    );
}

#[test]
fn every_remote_project_operation_is_reachable_from_a_public_tool() {
    let tools = EXPECTED_TOOL_NAMES.iter().copied().collect::<BTreeSet<_>>();
    let non_identity_tools = BTreeSet::from([
        "computer_activate",
        "computer_click",
        "computer_close",
        "computer_drag",
        "computer_launch",
        "computer_list_windows",
        "computer_press_keys",
        "computer_screenshot",
        "computer_scroll",
        "computer_set_clipboard",
        "computer_stop",
        "computer_type_text",
        "repo_diff",
    ]);
    for operation in ALL_PROJECT_OPERATION_KINDS {
        let public_name = match *operation {
            "computer_activate" => "activate_computer_window",
            "computer_click" => "click_computer_window",
            "computer_close" => "close_computer_window",
            "computer_drag" => "drag_computer_window",
            "computer_launch" => "launch_computer_app",
            "computer_list_windows" => "list_computer_windows",
            "computer_press_keys" => "press_computer_keys",
            "computer_screenshot" => "screenshot_computer_window",
            "computer_scroll" => "scroll_computer_window",
            "computer_set_clipboard" => "set_computer_clipboard",
            "computer_stop" => "stop_computer_control",
            "computer_type_text" => "type_computer_text",
            "repo_diff" => "repo_diff_summary",
            other if !non_identity_tools.contains(other) => other,
            other => panic!("unmapped operation: {other}"),
        };
        assert!(
            tools.contains(public_name),
            "missing public tool for {operation}"
        );
    }
}
