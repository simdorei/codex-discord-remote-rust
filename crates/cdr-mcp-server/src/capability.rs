use crate::scopes::{COMPUTER_CONTROL, COMPUTER_OBSERVE, FILES_READ, FILES_WRITE};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilitySurface {
    Read,
    Write,
    Git,
    ComputerObserve,
    ComputerControl,
    TerminalExecute,
    TerminalInteract,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityGroup {
    pub surface: CapabilitySurface,
    pub oauth_scopes: &'static [&'static str],
    pub tools: &'static [&'static str],
}

const READ_TOOLS: &[&str] = &[
    "capability_inventory",
    "checkpoint_list",
    "checkpoint_show",
    "code_search",
    "command_list",
    "device_info",
    "file_read_slice",
    "list_devices",
    "list_images",
    "list_project_files",
    "project_info",
    "project_rules",
    "project_status",
    "read_project_file",
    "repo_diff_summary",
    "repo_status",
    "retrieve_image",
    "select_project",
    "select_device",
    "set_working_directory",
    "show_changes",
];
const WRITE_TOOLS: &[&str] = &[
    "checkpoint_restore",
    "command_run",
    "file_apply_patch",
    "file_create",
    "save_image",
    "save_image_from_url",
    "write_project_file",
];
const GIT_TOOLS: &[&str] = &["git_commit", "git_push"];
const TERMINAL_EXECUTE_TOOLS: &[&str] = &[
    "terminal_exec",
    "terminal_window_close",
    "terminal_window_list",
    "terminal_window_open",
];
const TERMINAL_INTERACT_TOOLS: &[&str] = &[
    "terminal_window_activate",
    "terminal_window_capture",
    "terminal_window_interrupt",
    "terminal_window_keys",
    "terminal_window_type",
];
const COMPUTER_OBSERVE_TOOLS: &[&str] = &["list_computer_windows", "screenshot_computer_window"];
const COMPUTER_CONTROL_TOOLS: &[&str] = &[
    "activate_computer_window",
    "click_computer_window",
    "close_computer_window",
    "drag_computer_window",
    "launch_computer_app",
    "press_computer_keys",
    "scroll_computer_window",
    "set_computer_clipboard",
    "stop_computer_control",
    "type_computer_text",
];

pub const CAPABILITY_GROUPS: &[CapabilityGroup] = &[
    CapabilityGroup {
        surface: CapabilitySurface::Read,
        oauth_scopes: &[FILES_READ],
        tools: READ_TOOLS,
    },
    CapabilityGroup {
        surface: CapabilitySurface::Write,
        oauth_scopes: &[FILES_READ, FILES_WRITE],
        tools: WRITE_TOOLS,
    },
    CapabilityGroup {
        surface: CapabilitySurface::Git,
        oauth_scopes: &[FILES_READ, FILES_WRITE],
        tools: GIT_TOOLS,
    },
    CapabilityGroup {
        surface: CapabilitySurface::TerminalExecute,
        oauth_scopes: &[FILES_READ, FILES_WRITE],
        tools: TERMINAL_EXECUTE_TOOLS,
    },
    CapabilityGroup {
        surface: CapabilitySurface::TerminalInteract,
        oauth_scopes: &[FILES_READ, FILES_WRITE],
        tools: TERMINAL_INTERACT_TOOLS,
    },
    CapabilityGroup {
        surface: CapabilitySurface::ComputerObserve,
        oauth_scopes: &[FILES_READ, COMPUTER_OBSERVE],
        tools: COMPUTER_OBSERVE_TOOLS,
    },
    CapabilityGroup {
        surface: CapabilitySurface::ComputerControl,
        oauth_scopes: &[FILES_READ, COMPUTER_OBSERVE, COMPUTER_CONTROL],
        tools: COMPUTER_CONTROL_TOOLS,
    },
];

pub const EXPECTED_TOOL_NAMES: &[&str] = &[
    "activate_computer_window",
    "capability_inventory",
    "checkpoint_list",
    "checkpoint_restore",
    "checkpoint_show",
    "click_computer_window",
    "close_computer_window",
    "code_search",
    "command_list",
    "command_run",
    "device_info",
    "drag_computer_window",
    "file_apply_patch",
    "file_create",
    "file_read_slice",
    "git_commit",
    "git_push",
    "launch_computer_app",
    "list_computer_windows",
    "list_devices",
    "list_images",
    "list_project_files",
    "press_computer_keys",
    "project_info",
    "project_rules",
    "project_status",
    "read_project_file",
    "repo_diff_summary",
    "repo_status",
    "retrieve_image",
    "save_image",
    "save_image_from_url",
    "screenshot_computer_window",
    "scroll_computer_window",
    "select_device",
    "select_project",
    "set_computer_clipboard",
    "set_working_directory",
    "show_changes",
    "stop_computer_control",
    "terminal_exec",
    "terminal_window_activate",
    "terminal_window_capture",
    "terminal_window_close",
    "terminal_window_interrupt",
    "terminal_window_keys",
    "terminal_window_list",
    "terminal_window_open",
    "terminal_window_type",
    "type_computer_text",
    "write_project_file",
];

#[must_use]
pub fn required_scopes(tool: &str) -> Option<&'static [&'static str]> {
    CAPABILITY_GROUPS
        .iter()
        .find(|group| group.tools.contains(&tool))
        .map(|group| group.oauth_scopes)
}
