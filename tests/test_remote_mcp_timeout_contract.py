# pyright: reportUnnecessaryComparison=false
from __future__ import annotations

from datetime import UTC, datetime, timedelta
from typing import assert_never, final, override

import anyio
import pytest

from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.broker_models import BridgeSender
from simdorei_mcp_common.messages import (
    DeviceId,
    GatewayCommand,
    ListFilesCommand,
    ProjectInfoCommand,
    ProjectOperationCommand,
    ProjectOperationResult,
    ProjectSessionCommand,
    ProjectSessionResult,
    ProjectUpsert,
    ReadFileCommand,
    RequestId,
    WriteFileCommand,
)
from simdorei_mcp_common.operation_outputs import CommandRunOutput
from simdorei_mcp_common.operation_requests import (
    CommandRunRequest,
    FileCreateRequest,
    GitCommitRequest,
    GitPushRequest,
    ProjectOperation,
    ProjectStatusRequest,
    RepoDiffRequest,
    RepoStatusRequest,
)
from simdorei_mcp_common.request_deadlines import operation_request_deadline
from simdorei_mcp_common.terminal_protocol import TerminalExecRequest
from tests.remote_mcp_oauth_support import oauth_settings


@final
class CommandRunSender(BridgeSender):
    def __init__(self, broker: BindingBroker) -> None:
        self._broker: BindingBroker = broker
        self.command: ProjectOperationCommand | None = None

    @override
    async def send(self, command: GatewayCommand) -> None:
        match command:
            case ProjectSessionCommand():
                result = ProjectSessionResult(request_id=command.request_id)
            case ProjectOperationCommand():
                self.command = command
                result = ProjectOperationResult(
                    request_id=command.request_id,
                    output=CommandRunOutput(
                        command_id="qa",
                        exit_code=0,
                        stdout="",
                        stderr="",
                        duration_ms=1,
                        truncated=False,
                    ),
                )
            case (
                ProjectInfoCommand()
                | ListFilesCommand()
                | ReadFileCommand()
                | WriteFileCommand()
            ):
                raise AssertionError(f"unexpected command: {command.type}")
            case unreachable:
                assert_never(unreachable)
        await self._broker.complete(DeviceId("device-a"), self, result)

    @override
    async def close(self) -> None:
        return None


def test_gateway_default_covers_the_longest_terminal_command() -> None:
    settings = oauth_settings()

    assert settings.request_timeout_seconds >= 3_630


@pytest.mark.parametrize(
    ("operation", "expected_seconds"),
    (
        (FileCreateRequest(path="notes.txt", content="x"), 60),
        (RepoStatusRequest(), 135),
        (RepoDiffRequest(), 135),
        (GitCommitRequest(message="test", paths=("notes.txt",)), 135),
        (ProjectStatusRequest(), 135),
        (GitPushRequest(remote="origin", branch="main"), 315),
        (CommandRunRequest(command_id="qa", timeout_seconds=300), 315),
        (TerminalExecRequest(command="echo qa", timeout_seconds=3_600), 3_615),
    ),
)
def test_operation_deadline_matches_the_longest_bounded_local_work(
    operation: ProjectOperation,
    expected_seconds: int,
) -> None:
    before = datetime.now(UTC)
    deadline = operation_request_deadline(operation)
    after = datetime.now(UTC)

    assert deadline >= before + timedelta(seconds=expected_seconds - 0.1)
    assert deadline <= after + timedelta(seconds=expected_seconds + 0.1)


def test_command_deadline_and_stable_routed_request_id_cross_the_bridge(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    async def scenario() -> None:
        broker = BindingBroker()
        sender = CommandRunSender(broker)
        _ = await broker.attach(DeviceId("device-a"), sender)
        scope = "codex-pro-project-a"
        await broker.upsert(
            DeviceId("device-a"),
            sender,
            ProjectUpsert(
                project_scope=scope,
                binding_id="binding-generation-project-a",
                thread_id="thread-a",
                project_name="project-a",
                expires_at=datetime.now(UTC) + timedelta(minutes=10),
            ),
        )
        _ = await broker.select("session-a", "subject-a", scope)
        first_deadline = datetime.now(UTC) + timedelta(seconds=315)
        second_deadline = first_deadline + timedelta(seconds=1)
        deadlines = iter((first_deadline, second_deadline))
        def next_deadline(_operation: ProjectOperation) -> datetime:
            return next(deadlines)

        monkeypatch.setattr(
            "remote_mcp_server.simdorei_mcp.broker_requests.operation_request_deadline",
            next_deadline,
        )

        _ = await broker.project_operation(
            "session-a",
            "subject-a",
            CommandRunRequest(command_id="qa", timeout_seconds=300),
            request_id=RequestId("stable-external-request-id"),
        )

        assert sender.command is not None
        first_request_id = sender.command.request_id
        assert first_request_id != "stable-external-request-id"
        assert sender.command.deadline_at == first_deadline

        _ = await broker.project_operation(
            "session-a",
            "subject-a",
            CommandRunRequest(command_id="qa", timeout_seconds=300),
            request_id=RequestId("stable-external-request-id"),
        )

        assert sender.command.deadline_at == second_deadline
        assert sender.command.request_id == first_request_id

    anyio.run(scenario)
