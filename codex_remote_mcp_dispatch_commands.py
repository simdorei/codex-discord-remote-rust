# pyright: reportUnnecessaryComparison=false
from __future__ import annotations

from typing import assert_never

from codex_remote_mcp_computer import ComputerController
from codex_remote_mcp_files import ProjectFileAccess
from codex_remote_mcp_operations import (
    execute_project_operation,
    write_file_with_checkpoint,
)
from codex_remote_mcp_terminal_engine import TerminalExecutionEngine
from codex_remote_mcp_terminal_windows import TerminalWindowManager
from simdorei_mcp_common.messages import (
    BridgeResult,
    ListFilesCommand,
    ListFilesResult,
    ProjectInfoCommand,
    ProjectInfoResult,
    ProjectOperationCommand,
    ProjectOperationResult,
    ReadFileCommand,
    ReadFileResult,
    WriteFileCommand,
    WriteFileResult,
)
from simdorei_mcp_common.request_deadlines import RequestBudget

BoundProjectCommand = (
    ProjectInfoCommand
    | ListFilesCommand
    | ReadFileCommand
    | WriteFileCommand
    | ProjectOperationCommand
)


def execute_bound_project_command(
    command: BoundProjectCommand,
    access: ProjectFileAccess,
    computer: ComputerController | None,
    *,
    terminal: TerminalExecutionEngine | None = None,
    terminal_windows: TerminalWindowManager | None = None,
    budget: RequestBudget,
) -> BridgeResult:
    budget.ensure_active()
    access.verify_root()
    match command:
        case ProjectInfoCommand(thread_id=thread_id):
            return ProjectInfoResult(
                request_id=command.request_id,
                output=access.project_info(thread_id),
            )
        case ListFilesCommand(pattern=pattern, limit=limit):
            return ListFilesResult(
                request_id=command.request_id,
                output=access.list_files(
                    pattern,
                    limit,
                    ensure_active=budget.ensure_active,
                ),
            )
        case ReadFileCommand(path=path, start_line=start_line, max_lines=max_lines):
            return ReadFileResult(
                request_id=command.request_id,
                output=access.read_file(
                    path,
                    start_line=start_line,
                    max_lines=max_lines,
                ),
            )
        case WriteFileCommand(
            path=path,
            content=content,
            expected_sha256=expected_sha256,
        ):
            return WriteFileResult(
                request_id=command.request_id,
                output=write_file_with_checkpoint(
                    access.root,
                    path,
                    content,
                    expected_sha256=expected_sha256,
                    budget=budget,
                ),
            )
        case ProjectOperationCommand(operation=operation):
            return ProjectOperationResult(
                request_id=command.request_id,
                output=execute_project_operation(
                    access.root,
                    operation,
                    computer=computer,
                    terminal=terminal,
                    terminal_windows=terminal_windows,
                    budget=budget,
                ),
            )
        case unreachable:
            assert_never(unreachable)
