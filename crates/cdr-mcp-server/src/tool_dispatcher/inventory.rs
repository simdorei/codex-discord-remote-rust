use serde_json::json;

use crate::capability::{CAPABILITY_GROUPS, CapabilitySurface, EXPECTED_TOOL_NAMES};
use crate::mcp_http::DispatchOutput;

pub(super) fn capability_inventory() -> DispatchOutput {
    let groups = CAPABILITY_GROUPS
        .iter()
        .map(|group| {
            json!({
                "surface": surface(group.surface),
                "oauth_scopes": group.oauth_scopes,
                "tools": group.tools,
            })
        })
        .collect::<Vec<_>>();
    DispatchOutput::Structured(json!({
        "ready": true,
        "protocol_version": 10,
        "expected_tool_count": EXPECTED_TOOL_NAMES.len(),
        "registered_tool_count": EXPECTED_TOOL_NAMES.len(),
        "missing_tools": [],
        "unexpected_tools": [],
        "manifest_duplicate_tools": [],
        "groups": groups,
    }))
}

fn surface(value: CapabilitySurface) -> &'static str {
    match value {
        CapabilitySurface::Read => "read",
        CapabilitySurface::Write => "write",
        CapabilitySurface::Git => "git",
        CapabilitySurface::ComputerObserve => "computer_observe",
        CapabilitySurface::ComputerControl => "computer_control",
        CapabilitySurface::TerminalExecute => "terminal_execute",
        CapabilitySurface::TerminalInteract => "terminal_interact",
    }
}
