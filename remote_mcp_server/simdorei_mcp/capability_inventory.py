from __future__ import annotations

from collections import Counter
from collections.abc import Iterable
from enum import StrEnum, unique
from typing import ClassVar, Literal

from pydantic import BaseModel, ConfigDict

from remote_mcp_server.simdorei_mcp.oauth_scopes import (
    COMPUTER_CONTROL_REQUIRED_SCOPES,
    COMPUTER_OBSERVE_REQUIRED_SCOPES,
    READ_SCOPE,
    TERMINAL_EXECUTE_REQUIRED_SCOPES,
    TERMINAL_INTERACT_REQUIRED_SCOPES,
    WRITE_SCOPE,
)


@unique
class CapabilitySurface(StrEnum):
    READ = "read"
    WRITE = "write"
    GIT = "git"
    COMPUTER_OBSERVE = "computer_observe"
    COMPUTER_CONTROL = "computer_control"
    TERMINAL_EXECUTE = "terminal_execute"
    TERMINAL_INTERACT = "terminal_interact"


class CapabilityGroup(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)

    surface: CapabilitySurface
    oauth_scopes: tuple[str, ...]
    tools: tuple[str, ...]


class CapabilityInventoryOutput(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)

    ready: bool
    protocol_version: Literal[10] = 10
    expected_tool_count: int
    registered_tool_count: int
    missing_tools: tuple[str, ...]
    unexpected_tools: tuple[str, ...]
    manifest_duplicate_tools: tuple[str, ...]
    groups: tuple[CapabilityGroup, ...]


class ToolInventoryMismatch(RuntimeError):
    """Raised when FastMCP registration differs from the release manifest."""


CAPABILITY_GROUPS = (
    CapabilityGroup(
        surface=CapabilitySurface.READ,
        oauth_scopes=(READ_SCOPE,),
        tools=(
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
        ),
    ),
    CapabilityGroup(
        surface=CapabilitySurface.WRITE,
        oauth_scopes=(READ_SCOPE, WRITE_SCOPE),
        tools=(
            "checkpoint_restore",
            "command_run",
            "file_apply_patch",
            "file_create",
            "save_image",
            "save_image_from_url",
            "write_project_file",
        ),
    ),
    CapabilityGroup(
        surface=CapabilitySurface.GIT,
        oauth_scopes=(READ_SCOPE, WRITE_SCOPE),
        tools=("git_commit", "git_push"),
    ),
    CapabilityGroup(
        surface=CapabilitySurface.TERMINAL_EXECUTE,
        oauth_scopes=TERMINAL_EXECUTE_REQUIRED_SCOPES,
        tools=(
            "terminal_exec",
            "terminal_window_close",
            "terminal_window_list",
            "terminal_window_open",
        ),
    ),
    CapabilityGroup(
        surface=CapabilitySurface.TERMINAL_INTERACT,
        oauth_scopes=TERMINAL_INTERACT_REQUIRED_SCOPES,
        tools=(
            "terminal_window_activate",
            "terminal_window_capture",
            "terminal_window_interrupt",
            "terminal_window_keys",
            "terminal_window_type",
        ),
    ),
    CapabilityGroup(
        surface=CapabilitySurface.COMPUTER_OBSERVE,
        oauth_scopes=COMPUTER_OBSERVE_REQUIRED_SCOPES,
        tools=("list_computer_windows", "screenshot_computer_window"),
    ),
    CapabilityGroup(
        surface=CapabilitySurface.COMPUTER_CONTROL,
        oauth_scopes=COMPUTER_CONTROL_REQUIRED_SCOPES,
        tools=(
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
        ),
    ),
)
EXPECTED_TOOL_NAMES = tuple(
    sorted({tool_name for group in CAPABILITY_GROUPS for tool_name in group.tools})
)


def find_duplicate_tool_names(
    groups: Iterable[CapabilityGroup],
) -> tuple[str, ...]:
    counts = Counter(tool_name for group in groups for tool_name in group.tools)
    return tuple(sorted(name for name, count in counts.items() if count > 1))


def build_capability_inventory(
    registered_tool_names: Iterable[str],
) -> CapabilityInventoryOutput:
    registered = tuple(sorted(set(registered_tool_names)))
    expected = set(EXPECTED_TOOL_NAMES)
    actual = set(registered)
    missing = tuple(sorted(expected - actual))
    unexpected = tuple(sorted(actual - expected))
    manifest_duplicates = find_duplicate_tool_names(CAPABILITY_GROUPS)
    return CapabilityInventoryOutput(
        ready=not missing and not unexpected and not manifest_duplicates,
        expected_tool_count=len(EXPECTED_TOOL_NAMES),
        registered_tool_count=len(registered),
        missing_tools=missing,
        unexpected_tools=unexpected,
        manifest_duplicate_tools=manifest_duplicates,
        groups=CAPABILITY_GROUPS,
    )


def require_complete_tool_inventory(
    registered_tool_names: Iterable[str],
) -> CapabilityInventoryOutput:
    inventory = build_capability_inventory(registered_tool_names)
    if inventory.ready:
        return inventory
    message = (
        f"MCP tool inventory mismatch: missing={list(inventory.missing_tools)!r}, "
        f"unexpected={list(inventory.unexpected_tools)!r}, "
        "manifest_duplicates="
        f"{list(inventory.manifest_duplicate_tools)!r}"
    )
    raise ToolInventoryMismatch(message)
