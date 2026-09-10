from __future__ import annotations

from uuid import uuid4

from remote_mcp_server.simdorei_mcp.broker_idempotency import (
    derive_project_operation_request_id,
)
from remote_mcp_server.simdorei_mcp.broker_models import BridgeSender, SessionRoute
from remote_mcp_server.simdorei_mcp.broker_results import (
    list_files_output,
    operation_output,
    project_info_output,
    read_file_output,
    write_file_output,
)
from simdorei_mcp_common.messages import (
    BridgeResult,
    GatewayCommand,
    ListFilesCommand,
    ListFilesOutput,
    ProjectInfoCommand,
    ProjectInfoOutput,
    ProjectOperationCommand,
    ReadFileCommand,
    ReadFileOutput,
    RequestId,
    WriteFileCommand,
    WriteFileOutput,
)
from simdorei_mcp_common.operation_outputs import ProjectOperationOutput
from simdorei_mcp_common.operation_requests import ProjectOperation
from simdorei_mcp_common.request_deadlines import operation_request_deadline


class BrokerRequestsMixin:
    """Public project-operation facade shared by the synchronized broker."""

    async def _route(
        self,
        session: str,
        subject: str,
    ) -> tuple[SessionRoute, BridgeSender]:
        raise NotImplementedError

    async def _dispatch(
        self,
        route: SessionRoute,
        sender: BridgeSender,
        command: GatewayCommand,
    ) -> BridgeResult:
        raise NotImplementedError

    async def project_info(self, session: str, subject: str) -> ProjectInfoOutput:
        route, sender = await self._route(session, subject)
        result = await self._dispatch(
            route,
            sender,
            ProjectInfoCommand(
                request_id=RequestId(uuid4().hex),
                thread_id=route.thread_id,
                computer_session_id=route.computer_session_id,
            ),
        )
        return project_info_output(result)

    async def list_files(
        self,
        session: str,
        subject: str,
        *,
        pattern: str,
        limit: int,
    ) -> ListFilesOutput:
        route, sender = await self._route(session, subject)
        result = await self._dispatch(
            route,
            sender,
            ListFilesCommand(
                request_id=RequestId(uuid4().hex),
                thread_id=route.thread_id,
                computer_session_id=route.computer_session_id,
                pattern=pattern,
                limit=limit,
            ),
        )
        return list_files_output(result)

    async def read_file(
        self,
        session: str,
        subject: str,
        command: ReadFileCommand,
    ) -> ReadFileOutput:
        route, sender = await self._route(session, subject)
        routed = command.model_copy(
            update={
                "thread_id": route.thread_id,
                "computer_session_id": route.computer_session_id,
            }
        )
        return read_file_output(await self._dispatch(route, sender, routed))

    async def write_file(
        self,
        session: str,
        subject: str,
        command: WriteFileCommand,
    ) -> WriteFileOutput:
        route, sender = await self._route(session, subject)
        routed = command.model_copy(
            update={
                "thread_id": route.thread_id,
                "computer_session_id": route.computer_session_id,
            }
        )
        return write_file_output(await self._dispatch(route, sender, routed))

    async def project_operation(
        self,
        session: str,
        subject: str,
        operation: ProjectOperation,
        *,
        request_id: RequestId | None = None,
    ) -> ProjectOperationOutput:
        route, sender = await self._route(session, subject)
        base_request_id = request_id or RequestId(uuid4().hex)
        command = ProjectOperationCommand(
            request_id=base_request_id,
            thread_id=route.thread_id,
            computer_session_id=route.computer_session_id,
            deadline_at=operation_request_deadline(operation),
            operation=operation,
        )
        routed = command.model_copy(
            update={
                "request_id": derive_project_operation_request_id(
                    base_request_id,
                    command,
                )
            }
        )
        result = await self._dispatch(route, sender, routed)
        return operation_output(result)
