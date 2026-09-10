from __future__ import annotations

from datetime import datetime
from typing import Annotated, ClassVar, Literal, NewType, Self

from pydantic import BaseModel, ConfigDict, Field, TypeAdapter, model_validator
from pydantic_core import PydanticCustomError

from simdorei_mcp_common.operation_outputs import ProjectOperationOutput
from simdorei_mcp_common.operation_requests import ProjectOperation
from simdorei_mcp_common.request_deadlines import default_request_deadline

DeviceId = NewType("DeviceId", str)
RequestId = NewType("RequestId", str)


class ProtocolModel(BaseModel):
    """Immutable base for values crossing the bridge boundary."""

    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")


class ProjectUpsert(ProtocolModel):
    type: Literal["project_upsert"] = "project_upsert"
    project_scope: str = Field(min_length=12, max_length=200)
    binding_id: str = Field(min_length=16, max_length=64)
    thread_id: str = Field(min_length=1, max_length=200)
    project_name: str = Field(min_length=1, max_length=200)
    expires_at: datetime


class ProjectSelectionOutput(ProtocolModel):
    project_name: str
    thread_id: str
    expires_at: datetime


class ProjectInfoOutput(ProtocolModel):
    root: str
    thread_id: str


class FileEntry(ProtocolModel):
    path: str
    size_bytes: int = Field(ge=0)


class ListFilesOutput(ProtocolModel):
    files: tuple[FileEntry, ...]
    truncated: bool


class ReadFileOutput(ProtocolModel):
    path: str
    content: str
    sha256: str
    start_line: int = Field(ge=1)
    end_line: int = Field(ge=0)
    total_lines: int = Field(ge=0)
    truncated: bool
    redacted: bool


class WriteFileOutput(ProtocolModel):
    path: str
    sha256: str
    bytes_written: int = Field(ge=0)
    created: bool


class ProjectCommand(ProtocolModel):
    """Base class for commands routed through one selected ChatGPT session."""

    request_id: RequestId
    thread_id: str
    deadline_at: datetime = Field(default_factory=default_request_deadline)
    computer_session_id: str | None = Field(
        default=None,
        min_length=16,
        max_length=64,
    )


class ProjectInfoCommand(ProjectCommand):
    type: Literal["project_info"] = "project_info"


class ListFilesCommand(ProjectCommand):
    type: Literal["list_files"] = "list_files"
    pattern: str = Field(min_length=1, max_length=500)
    limit: int = Field(ge=1, le=500)


class ReadFileCommand(ProjectCommand):
    type: Literal["read_file"] = "read_file"
    path: str = Field(min_length=1, max_length=1_000)
    start_line: int = Field(ge=1)
    max_lines: int = Field(ge=1, le=500)


class WriteFileCommand(ProjectCommand):
    type: Literal["write_file"] = "write_file"
    path: str = Field(min_length=1, max_length=1_000)
    content: str = Field(max_length=1_048_576)
    expected_sha256: str | None = Field(default=None, min_length=64, max_length=64)


class ProjectOperationCommand(ProjectCommand):
    type: Literal["project_operation"] = "project_operation"
    operation: ProjectOperation


class ProjectSessionCommand(ProjectCommand):
    """Makes one computer-control generation authoritative for a thread."""

    type: Literal["project_session"] = "project_session"
    computer_session_generation: int = Field(ge=1)

    @model_validator(mode="after")
    def require_session_generation(self) -> Self:
        if self.computer_session_id is None:
            raise PydanticCustomError(
                "computer_session_id",
                "computer_session_id is required",
            )
        return self


class DeviceSessionCommand(ProjectCommand):
    type: Literal["device_session"] = "device_session"
    computer_session_generation: int = Field(ge=1)
    working_directory: str = Field(min_length=1, max_length=1_000)
    expires_at: datetime

    @model_validator(mode="after")
    def require_session_generation(self) -> Self:
        if self.computer_session_id is None:
            raise PydanticCustomError(
                "computer_session_id",
                "computer_session_id is required",
            )
        return self


class ProjectInfoResult(ProtocolModel):
    type: Literal["project_info_result"] = "project_info_result"
    request_id: RequestId
    output: ProjectInfoOutput


class ListFilesResult(ProtocolModel):
    type: Literal["list_files_result"] = "list_files_result"
    request_id: RequestId
    output: ListFilesOutput


class ReadFileResult(ProtocolModel):
    type: Literal["read_file_result"] = "read_file_result"
    request_id: RequestId
    output: ReadFileOutput


class WriteFileResult(ProtocolModel):
    type: Literal["write_file_result"] = "write_file_result"
    request_id: RequestId
    output: WriteFileOutput


class ProjectOperationResult(ProtocolModel):
    type: Literal["project_operation_result"] = "project_operation_result"
    request_id: RequestId
    output: ProjectOperationOutput


class ProjectSessionResult(ProtocolModel):
    type: Literal["project_session_result"] = "project_session_result"
    request_id: RequestId


class OperationErrorResult(ProtocolModel):
    type: Literal["operation_error"] = "operation_error"
    request_id: RequestId
    error_code: str = Field(min_length=1, max_length=100)
    message: str = Field(min_length=1, max_length=1_000)


class BridgeHello(ProtocolModel):
    type: Literal["hello"] = "hello"
    protocol_version: Literal[10]
    device_id: DeviceId


class GatewayHello(ProtocolModel):
    type: Literal["hello_ack"] = "hello_ack"
    protocol_version: Literal[10] = 10


class ProjectAck(ProtocolModel):
    type: Literal["project_ack"] = "project_ack"
    project_scope: str
    binding_id: str = Field(min_length=16, max_length=64)


GatewayCommand = Annotated[
    ProjectInfoCommand
    | ListFilesCommand
    | ReadFileCommand
    | WriteFileCommand
    | ProjectOperationCommand
    | ProjectSessionCommand
    | DeviceSessionCommand,
    Field(discriminator="type"),
]
BridgeResult = Annotated[
    ProjectInfoResult
    | ListFilesResult
    | ReadFileResult
    | WriteFileResult
    | ProjectOperationResult
    | ProjectSessionResult
    | OperationErrorResult,
    Field(discriminator="type"),
]
BridgeInboundMessage = Annotated[
    BridgeHello
    | ProjectUpsert
    | ProjectInfoResult
    | ListFilesResult
    | ReadFileResult
    | WriteFileResult
    | ProjectOperationResult
    | ProjectSessionResult
    | OperationErrorResult,
    Field(discriminator="type"),
]
GatewayInboundMessage = Annotated[
    GatewayHello
    | ProjectAck
    | ProjectInfoCommand
    | ListFilesCommand
    | ReadFileCommand
    | WriteFileCommand
    | ProjectOperationCommand
    | ProjectSessionCommand
    | DeviceSessionCommand,
    Field(discriminator="type"),
]

BRIDGE_INBOUND_ADAPTER: TypeAdapter[BridgeInboundMessage] = TypeAdapter(
    BridgeInboundMessage
)
GATEWAY_INBOUND_ADAPTER: TypeAdapter[GatewayInboundMessage] = TypeAdapter(
    GatewayInboundMessage
)


def parse_bridge_message(raw: str) -> BridgeInboundMessage:
    return BRIDGE_INBOUND_ADAPTER.validate_json(raw)


def parse_gateway_message(raw: str) -> GatewayInboundMessage:
    return GATEWAY_INBOUND_ADAPTER.validate_json(raw)
