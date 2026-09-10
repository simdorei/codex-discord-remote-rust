from __future__ import annotations

from typing import Annotated, Literal

from pydantic import Field

from simdorei_mcp_common.operation_base import OperationOutput
from simdorei_mcp_common.terminal_window_interaction_protocol import (
    TerminalWindowInteractionOutput,
)
from simdorei_mcp_common.terminal_window_protocol import TerminalOperationOutput


class RuleFile(OperationOutput):
    path: str
    content: str


class ProjectRulesOutput(OperationOutput):
    kind: Literal["project_rules"] = "project_rules"
    rules: tuple[RuleFile, ...]


class ProjectStatusOutput(OperationOutput):
    kind: Literal["project_status"] = "project_status"
    branch: str
    dirty_files: tuple[str, ...]
    staged_files: tuple[str, ...]
    rule_files: tuple[str, ...]
    command_ids: tuple[str, ...]


class SearchMatch(OperationOutput):
    path: str
    line: int = Field(ge=1)
    snippet: str


class CodeSearchOutput(OperationOutput):
    kind: Literal["code_search"] = "code_search"
    matches: tuple[SearchMatch, ...]


class PatchEntry(OperationOutput):
    path: str
    action: Literal["add", "update", "delete", "move"]
    destination: str | None = None
    added_lines: int = Field(ge=0)
    removed_lines: int = Field(ge=0)


class FileApplyPatchOutput(OperationOutput):
    kind: Literal["file_apply_patch"] = "file_apply_patch"
    applied: tuple[PatchEntry, ...]
    checkpoint_id: str


class FileCreateOutput(OperationOutput):
    kind: Literal["file_create"] = "file_create"
    path: str
    sha256: str
    bytes_written: int = Field(ge=0)
    checkpoint_id: str


class CommandDescriptor(OperationOutput):
    command_id: str
    display: str
    source: str
    risk_tier: Literal["read", "verify", "network", "destructive"]


class CommandListOutput(OperationOutput):
    kind: Literal["command_list"] = "command_list"
    commands: tuple[CommandDescriptor, ...]


class CommandRunOutput(OperationOutput):
    kind: Literal["command_run"] = "command_run"
    command_id: str
    exit_code: int
    stdout: str
    stderr: str
    duration_ms: int = Field(ge=0)
    truncated: bool


class RepoStatusOutput(OperationOutput):
    kind: Literal["repo_status"] = "repo_status"
    branch: str
    dirty_files: tuple[str, ...]
    staged_files: tuple[str, ...]
    remotes: tuple[str, ...]
    upstream: str | None
    ahead: int = Field(ge=0)
    behind: int = Field(ge=0)


class DiffFile(OperationOutput):
    path: str
    added: int = Field(ge=0)
    removed: int = Field(ge=0)


class RepoDiffOutput(OperationOutput):
    kind: Literal["repo_diff"] = "repo_diff"
    files: tuple[DiffFile, ...]
    summary: str
    patch: str
    truncated: bool


class GitCommitOutput(OperationOutput):
    kind: Literal["git_commit"] = "git_commit"
    commit: str
    branch: str
    staged_files: tuple[str, ...]


class GitPushOutput(OperationOutput):
    kind: Literal["git_push"] = "git_push"
    remote: str
    branch: str
    output: str


class ImageEntry(OperationOutput):
    path: str
    media_type: str
    size_bytes: int = Field(ge=0)


class ImageSaveOutput(OperationOutput):
    kind: Literal["image_save"] = "image_save"
    image: ImageEntry
    sha256: str


class ImageListOutput(OperationOutput):
    kind: Literal["image_list"] = "image_list"
    images: tuple[ImageEntry, ...]


class ImageRetrieveOutput(OperationOutput):
    kind: Literal["image_retrieve"] = "image_retrieve"
    image: ImageEntry
    data_base64: str


class CheckpointEntry(OperationOutput):
    checkpoint_id: str
    created_at: str
    reason: str


class CheckpointListOutput(OperationOutput):
    kind: Literal["checkpoint_list"] = "checkpoint_list"
    checkpoints: tuple[CheckpointEntry, ...]


class CheckpointShowOutput(OperationOutput):
    kind: Literal["checkpoint_show"] = "checkpoint_show"
    checkpoint: CheckpointEntry
    patch: str


class CheckpointRestoreOutput(OperationOutput):
    kind: Literal["checkpoint_restore"] = "checkpoint_restore"
    checkpoint_id: str
    restored_files: tuple[str, ...]


class ComputerWindowEntry(OperationOutput):
    window_id: int = Field(gt=0)
    title: str
    process_name: str
    left: int
    top: int
    width: int = Field(gt=0)
    height: int = Field(gt=0)
    active: bool


class ComputerWindowsOutput(OperationOutput):
    kind: Literal["computer_windows"] = "computer_windows"
    windows: tuple[ComputerWindowEntry, ...]


class ComputerScreenshotOutput(OperationOutput):
    kind: Literal["computer_screenshot"] = "computer_screenshot"
    observation_id: str = Field(min_length=8, max_length=100)
    window: ComputerWindowEntry
    media_type: Literal["image/png"] = "image/png"
    data_base64: str = Field(min_length=12, max_length=12_000_000)


ComputerActionName = Literal[
    "activate",
    "launch",
    "click",
    "drag",
    "scroll",
    "type_text",
    "press_keys",
    "close",
    "set_clipboard",
]


class ComputerActionOutput(OperationOutput):
    kind: Literal["computer_action"] = "computer_action"
    action: ComputerActionName
    window_id: int | None = Field(default=None, gt=0)
    message: str


class ComputerStopOutput(OperationOutput):
    kind: Literal["computer_stop"] = "computer_stop"
    stopped: Literal[True] = True
    message: str


ProjectOperationOutput = Annotated[
    ProjectRulesOutput
    | ProjectStatusOutput
    | CodeSearchOutput
    | FileApplyPatchOutput
    | FileCreateOutput
    | CommandListOutput
    | CommandRunOutput
    | RepoStatusOutput
    | RepoDiffOutput
    | GitCommitOutput
    | GitPushOutput
    | ImageSaveOutput
    | ImageListOutput
    | ImageRetrieveOutput
    | CheckpointListOutput
    | CheckpointShowOutput
    | CheckpointRestoreOutput
    | ComputerWindowsOutput
    | ComputerScreenshotOutput
    | ComputerActionOutput
    | ComputerStopOutput
    | TerminalOperationOutput
    | TerminalWindowInteractionOutput,
    Field(discriminator="kind"),
]
