from __future__ import annotations

from typing import assert_never

from remote_mcp_server.simdorei_mcp.broker_errors import (
    BridgeProtocolError,
    RemoteOperationError,
)
from simdorei_mcp_common.messages import (
    BridgeResult,
    ListFilesOutput,
    ListFilesResult,
    OperationErrorResult,
    ProjectInfoOutput,
    ProjectInfoResult,
    ProjectOperationResult,
    ProjectSessionResult,
    ReadFileOutput,
    ReadFileResult,
    WriteFileOutput,
    WriteFileResult,
)
from simdorei_mcp_common.operation_outputs import ProjectOperationOutput


def project_info_output(result: BridgeResult) -> ProjectInfoOutput:
    match result:
        case ProjectInfoResult(output=output):
            return output
        case OperationErrorResult(message=message):
            raise RemoteOperationError(message)
        case (
            ListFilesResult()
            | ReadFileResult()
            | WriteFileResult()
            | ProjectOperationResult()
            | ProjectSessionResult()
        ):
            raise BridgeProtocolError("local bridge returned the wrong result type")
    assert_never(result)


def list_files_output(result: BridgeResult) -> ListFilesOutput:
    match result:
        case ListFilesResult(output=output):
            return output
        case OperationErrorResult(message=message):
            raise RemoteOperationError(message)
        case (
            ProjectInfoResult()
            | ReadFileResult()
            | WriteFileResult()
            | ProjectOperationResult()
            | ProjectSessionResult()
        ):
            raise BridgeProtocolError("local bridge returned the wrong result type")
    assert_never(result)


def read_file_output(result: BridgeResult) -> ReadFileOutput:
    match result:
        case ReadFileResult(output=output):
            return output
        case OperationErrorResult(message=message):
            raise RemoteOperationError(message)
        case (
            ProjectInfoResult()
            | ListFilesResult()
            | WriteFileResult()
            | ProjectOperationResult()
            | ProjectSessionResult()
        ):
            raise BridgeProtocolError("local bridge returned the wrong result type")
    assert_never(result)


def write_file_output(result: BridgeResult) -> WriteFileOutput:
    match result:
        case WriteFileResult(output=output):
            return output
        case OperationErrorResult(message=message):
            raise RemoteOperationError(message)
        case (
            ProjectInfoResult()
            | ListFilesResult()
            | ReadFileResult()
            | ProjectOperationResult()
            | ProjectSessionResult()
        ):
            raise BridgeProtocolError("local bridge returned the wrong result type")
    assert_never(result)


def operation_output(result: BridgeResult) -> ProjectOperationOutput:
    match result:
        case ProjectOperationResult(output=output):
            return output
        case OperationErrorResult(message=message):
            raise RemoteOperationError(message)
        case (
            ProjectInfoResult()
            | ListFilesResult()
            | ReadFileResult()
            | WriteFileResult()
            | ProjectSessionResult()
        ):
            raise BridgeProtocolError("local bridge returned the wrong result type")
    assert_never(result)


def require_project_session_result(result: BridgeResult) -> None:
    match result:
        case ProjectSessionResult():
            return
        case OperationErrorResult(message=message):
            raise RemoteOperationError(message)
        case (
            ProjectInfoResult()
            | ListFilesResult()
            | ReadFileResult()
            | WriteFileResult()
            | ProjectOperationResult()
        ):
            raise BridgeProtocolError("local bridge returned the wrong result type")
    assert_never(result)
