# pyright: reportUnnecessaryComparison=false
from __future__ import annotations

from pathlib import Path
from typing import assert_never

from codex_remote_mcp_checkpoint_transaction import (
    CheckpointTarget,
    checkpoint_transaction,
)
from codex_remote_mcp_checkpoints import (
    begin_checkpoint,
    finish_checkpoint,
    list_checkpoints,
    restore_checkpoint,
    show_checkpoint,
)
from codex_remote_mcp_code import read_project_rules, search_project
from codex_remote_mcp_commands import list_commands, run_command
from codex_remote_mcp_computer import (
    ComputerController,
    execute_computer_operation,
)
from codex_remote_mcp_files import ProjectFileAccess, ProjectFileError
from codex_remote_mcp_git import git_commit, git_push, repo_diff, repo_status
from codex_remote_mcp_images import (
    list_images,
    retrieve_image,
    save_image,
    save_image_from_url,
)
from codex_remote_mcp_patch import apply_project_patch
from codex_remote_mcp_terminal_engine import (
    TerminalExecutionEngine,
)
from codex_remote_mcp_terminal_windows import TerminalWindowManager
from simdorei_mcp_common.messages import WriteFileOutput
from simdorei_mcp_common.operation_outputs import (
    CheckpointListOutput,
    CheckpointRestoreOutput,
    CheckpointShowOutput,
    FileCreateOutput,
    ProjectOperationOutput,
    ProjectStatusOutput,
)
from simdorei_mcp_common.operation_requests import (
    CheckpointListRequest,
    CheckpointRestoreRequest,
    CheckpointShowRequest,
    CodeSearchRequest,
    CommandListRequest,
    CommandRunRequest,
    ComputerActivateRequest,
    ComputerClickRequest,
    ComputerCloseRequest,
    ComputerDragRequest,
    ComputerLaunchRequest,
    ComputerListWindowsRequest,
    ComputerPressKeysRequest,
    ComputerScreenshotRequest,
    ComputerScrollRequest,
    ComputerSetClipboardRequest,
    ComputerStopRequest,
    ComputerTypeTextRequest,
    FileApplyPatchRequest,
    FileCreateRequest,
    GitCommitRequest,
    GitPushRequest,
    ListImagesRequest,
    ProjectOperation,
    ProjectRulesRequest,
    ProjectStatusRequest,
    RepoDiffRequest,
    RepoStatusRequest,
    RetrieveImageRequest,
    SaveImageFromUrlRequest,
    SaveImageRequest,
)
from simdorei_mcp_common.request_deadlines import RequestBudget
from simdorei_mcp_common.terminal_protocol import TerminalExecRequest
from simdorei_mcp_common.terminal_window_interaction_protocol import (
    TerminalWindowActivateRequest,
    TerminalWindowCaptureRequest,
    TerminalWindowInterruptRequest,
    TerminalWindowKeysRequest,
    TerminalWindowTypeRequest,
)
from simdorei_mcp_common.terminal_window_protocol import (
    TerminalWindowCloseRequest,
    TerminalWindowListRequest,
    TerminalWindowOpenRequest,
)


class ProjectCapabilityError(ProjectFileError):
    """Raised when a project capability cannot be executed."""


def execute_project_operation(
    root: Path,
    operation: ProjectOperation,
    *,
    computer: ComputerController | None = None,
    terminal: TerminalExecutionEngine | None = None,
    terminal_windows: TerminalWindowManager | None = None,
    budget: RequestBudget,
) -> ProjectOperationOutput:
    """Execute one typed capability inside a validated project root."""
    budget.ensure_active()
    match operation:
        case TerminalExecRequest():
            if terminal is None:
                raise ProjectCapabilityError(
                    "terminal",
                    "terminal execution is unavailable for this project session",
                )
            output = terminal.execute(
                operation,
                cancel_event=budget.cancel_event,
                timeout_seconds=budget.remaining(operation.timeout_seconds),
            )
            budget.ensure_active()
            return output
        case TerminalWindowOpenRequest():
            if terminal_windows is None:
                raise ProjectCapabilityError(
                    "terminal",
                    "terminal windows are unavailable for this project session",
                )
            output = terminal_windows.open(operation)
            budget.ensure_active()
            return output
        case TerminalWindowListRequest():
            if terminal_windows is None:
                raise ProjectCapabilityError(
                    "terminal",
                    "terminal windows are unavailable for this project session",
                )
            output = terminal_windows.list()
            budget.ensure_active()
            return output
        case TerminalWindowCloseRequest():
            if terminal_windows is None:
                raise ProjectCapabilityError(
                    "terminal",
                    "terminal windows are unavailable for this project session",
                )
            output = terminal_windows.close(operation)
            budget.ensure_active()
            return output
        case TerminalWindowCaptureRequest():
            if terminal_windows is None:
                raise ProjectCapabilityError(
                    "terminal",
                    "terminal windows are unavailable for this project session",
                )
            output = terminal_windows.capture(operation)
            budget.ensure_active()
            return output
        case TerminalWindowActivateRequest():
            if terminal_windows is None:
                raise ProjectCapabilityError(
                    "terminal",
                    "terminal windows are unavailable for this project session",
                )
            output = terminal_windows.activate(operation)
            budget.ensure_active()
            return output
        case TerminalWindowTypeRequest():
            if terminal_windows is None:
                raise ProjectCapabilityError(
                    "terminal",
                    "terminal windows are unavailable for this project session",
                )
            output = terminal_windows.type_text(operation)
            budget.ensure_active()
            return output
        case TerminalWindowKeysRequest():
            if terminal_windows is None:
                raise ProjectCapabilityError(
                    "terminal",
                    "terminal windows are unavailable for this project session",
                )
            output = terminal_windows.press_keys(operation)
            budget.ensure_active()
            return output
        case TerminalWindowInterruptRequest():
            if terminal_windows is None:
                raise ProjectCapabilityError(
                    "terminal",
                    "terminal windows are unavailable for this project session",
                )
            output = terminal_windows.interrupt(operation)
            budget.ensure_active()
            return output
        case (
            ComputerListWindowsRequest()
            | ComputerActivateRequest()
            | ComputerLaunchRequest()
            | ComputerScreenshotRequest()
            | ComputerClickRequest()
            | ComputerDragRequest()
            | ComputerScrollRequest()
            | ComputerTypeTextRequest()
            | ComputerPressKeysRequest()
            | ComputerCloseRequest()
            | ComputerSetClipboardRequest()
            | ComputerStopRequest()
        ):
            return execute_computer_operation(
                operation,
                controller=computer,
                budget=budget,
            )
        case ProjectRulesRequest():
            return read_project_rules(root, budget=budget)
        case CodeSearchRequest():
            return search_project(root, operation, budget=budget)
        case CommandListRequest():
            return list_commands(root, budget=budget)
        case CommandRunRequest():
            return run_command(root, operation, budget=budget)
        case FileCreateRequest():
            return _create_file(root, operation, budget=budget)
        case FileApplyPatchRequest():
            return apply_project_patch(root, operation, budget=budget)
        case RepoStatusRequest():
            return repo_status(root, budget=budget)
        case RepoDiffRequest():
            return repo_diff(root, budget=budget)
        case GitCommitRequest():
            return git_commit(root, operation, budget=budget)
        case GitPushRequest():
            return git_push(root, operation, budget=budget)
        case SaveImageRequest():
            return save_image(
                root,
                operation.path,
                operation.data_base64,
                overwrite=operation.overwrite,
                budget=budget,
            )
        case SaveImageFromUrlRequest():
            return save_image_from_url(
                root,
                operation.path,
                str(operation.url),
                overwrite=operation.overwrite,
                budget=budget,
            )
        case ListImagesRequest():
            return list_images(root, budget=budget)
        case RetrieveImageRequest():
            return retrieve_image(root, operation.path, budget=budget)
        case ProjectStatusRequest():
            return _project_status(root, budget=budget)
        case CheckpointListRequest():
            return CheckpointListOutput(
                checkpoints=list_checkpoints(root, budget=budget)
            )
        case CheckpointShowRequest():
            checkpoint, patch = show_checkpoint(
                root,
                operation.checkpoint_id,
                budget=budget,
            )
            return CheckpointShowOutput(checkpoint=checkpoint, patch=patch)
        case CheckpointRestoreRequest():
            restored = restore_checkpoint(
                root,
                operation.checkpoint_id,
                budget=budget,
            )
            return CheckpointRestoreOutput(
                checkpoint_id=operation.checkpoint_id,
                restored_files=restored,
            )
        case unreachable:
            assert_never(unreachable)


def _project_status(root: Path, *, budget: RequestBudget) -> ProjectStatusOutput:
    status = repo_status(root, budget=budget)
    budget.ensure_active()
    rules = read_project_rules(root, budget=budget)
    budget.ensure_active()
    commands = list_commands(root, budget=budget)
    return ProjectStatusOutput(
        branch=status.branch,
        dirty_files=status.dirty_files,
        staged_files=status.staged_files,
        rule_files=tuple(rule.path for rule in rules.rules),
        command_ids=tuple(command.command_id for command in commands.commands),
    )


def _create_file(
    root: Path,
    request: FileCreateRequest,
    *,
    budget: RequestBudget,
) -> FileCreateOutput:
    access = ProjectFileAccess(root)
    target = access.resolve_path(request.path, require_file=False)
    if access.file_exists(request.path) and not request.overwrite:
        raise ProjectCapabilityError(request.path, "file already exists")
    budget.ensure_active()
    draft = begin_checkpoint(
        root,
        "create",
        (CheckpointTarget(path=request.path, absolute_path=target),),
    )
    expected_sha256 = None
    if access.file_exists(request.path):
        expected_sha256 = access.read_file(
            request.path,
            start_line=1,
            max_lines=1,
        ).sha256
    with checkpoint_transaction(draft) as transaction:
        budget.ensure_active()
        written = access.write_file(
            request.path,
            request.content,
            expected_sha256=expected_sha256,
        )
        transaction.record_write(request.path, written.sha256)
        checkpoint_id = finish_checkpoint(draft)
    return FileCreateOutput(
        path=written.path,
        sha256=written.sha256,
        bytes_written=written.bytes_written,
        checkpoint_id=checkpoint_id,
    )


def write_file_with_checkpoint(
    root: Path,
    path: str,
    content: str,
    *,
    expected_sha256: str | None,
    budget: RequestBudget,
) -> WriteFileOutput:
    """Preserve compatibility for the legacy write tool without losing undo."""
    access = ProjectFileAccess(root)
    target = access.resolve_path(path, require_file=False)
    budget.ensure_active()
    draft = begin_checkpoint(
        root,
        "write file",
        (CheckpointTarget(path=path, absolute_path=target),),
    )
    with checkpoint_transaction(draft) as transaction:
        budget.ensure_active()
        output = access.write_file(
            path,
            content,
            expected_sha256=expected_sha256,
        )
        transaction.record_write(path, output.sha256)
        _ = finish_checkpoint(draft)
    return output
