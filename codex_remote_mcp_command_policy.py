# pyright: reportUnnecessaryComparison=false
from __future__ import annotations

from typing import assert_never

from simdorei_mcp_common.messages import (
    DeviceSessionCommand,
    GatewayCommand,
    ListFilesCommand,
    ProjectInfoCommand,
    ProjectOperationCommand,
    ProjectSessionCommand,
    ReadFileCommand,
    WriteFileCommand,
)
from simdorei_mcp_common.operation_requests import (
    CheckpointListRequest,
    CheckpointShowRequest,
    CodeSearchRequest,
    CommandListRequest,
    ListImagesRequest,
    ProjectRulesRequest,
    ProjectStatusRequest,
    RepoDiffRequest,
    RepoStatusRequest,
    RetrieveImageRequest,
)
from simdorei_mcp_common.terminal_protocol import TerminalExecRequest


READ_ONLY_OPERATIONS = (
    ProjectRulesRequest,
    ProjectStatusRequest,
    CodeSearchRequest,
    CommandListRequest,
    RepoStatusRequest,
    RepoDiffRequest,
    ListImagesRequest,
    RetrieveImageRequest,
    CheckpointListRequest,
    CheckpointShowRequest,
)


def requires_execution_lock(command: GatewayCommand) -> bool:
    match command:
        case ProjectInfoCommand() | ListFilesCommand() | ReadFileCommand():
            return False
        case ProjectOperationCommand(operation=operation):
            if isinstance(operation, TerminalExecRequest):
                return False
            return not isinstance(operation, READ_ONLY_OPERATIONS)
        case WriteFileCommand() | ProjectSessionCommand() | DeviceSessionCommand():
            return True
        case unreachable:
            assert_never(unreachable)


__all__ = ["requires_execution_lock"]
