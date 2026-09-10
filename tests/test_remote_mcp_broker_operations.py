from __future__ import annotations

from datetime import UTC, datetime, timedelta
from typing import assert_never

import anyio

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
from simdorei_mcp_common.operation_outputs import RepoStatusOutput
from simdorei_mcp_common.operation_requests import (
    RepoDiffRequest,
    RepoStatusRequest,
)


class OperationSender(BridgeSender):
    def __init__(self, broker: BindingBroker) -> None:
        self._broker = broker
        self.command: ProjectOperationCommand | None = None
        self.commands: list[ProjectOperationCommand] = []

    async def send(self, command: GatewayCommand) -> None:
        match command:
            case ProjectOperationCommand():
                self.command = command
                self.commands.append(command)
                result = ProjectOperationResult(
                    request_id=command.request_id,
                    output=RepoStatusOutput(
                        branch="main",
                        dirty_files=(),
                        staged_files=(),
                        remotes=("origin",),
                        upstream="origin/main",
                        ahead=0,
                        behind=0,
                    ),
                )
            case ProjectSessionCommand():
                result = ProjectSessionResult(request_id=command.request_id)
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

    async def close(self) -> None:
        return None


class DelayedOperationSender(OperationSender):
    def __init__(self, broker: BindingBroker) -> None:
        super().__init__(broker)
        self.operation_started = anyio.Event()
        self.release_operation = anyio.Event()
        self.send_count = 0

    async def send(self, command: GatewayCommand) -> None:
        if isinstance(command, ProjectOperationCommand):
            self.send_count += 1
            self.operation_started.set()
            await self.release_operation.wait()
        await super().send(command)


def test_project_operation_is_forwarded_to_selected_local_thread() -> None:
    async def scenario() -> None:
        # Given
        broker = BindingBroker()
        sender = OperationSender(broker)
        await broker.attach(DeviceId("device-a"), sender)
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
        await broker.select("session-a", "subject-a", scope)

        # When
        output = await broker.project_operation(
            "session-a",
            "subject-a",
            RepoStatusRequest(),
        )

        # Then
        assert isinstance(output, RepoStatusOutput)
        assert sender.command is not None
        assert sender.command.thread_id == "thread-a"
        assert sender.command.computer_session_id is not None

    anyio.run(scenario)


def test_new_chat_owner_gets_a_new_computer_session_generation() -> None:
    async def scenario() -> None:
        broker = BindingBroker()
        sender = OperationSender(broker)
        await broker.attach(DeviceId("device-a"), sender)
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
        await broker.select("session-a", "subject-a", scope)
        _ = await broker.project_operation(
            "session-a",
            "subject-a",
            RepoStatusRequest(),
        )

        await broker.select("session-b", "subject-a", scope)
        _ = await broker.project_operation(
            "session-b",
            "subject-a",
            RepoStatusRequest(),
        )

        generations = [command.computer_session_id for command in sender.commands]
        assert None not in generations
        assert generations[0] != generations[1]

    anyio.run(scenario)


def test_duplicate_inflight_request_is_sent_to_the_bridge_once() -> None:
    async def scenario() -> None:
        broker = BindingBroker()
        sender = DelayedOperationSender(broker)
        await broker.attach(DeviceId("device-a"), sender)
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
        results: list[RepoStatusOutput] = []

        async def request_status() -> None:
            output = await broker.project_operation(
                "session-a",
                "subject-a",
                RepoStatusRequest(),
                request_id=RequestId("same-external-request"),
            )
            assert isinstance(output, RepoStatusOutput)
            results.append(output)

        async with anyio.create_task_group() as tasks:
            tasks.start_soon(request_status)
            await sender.operation_started.wait()
            tasks.start_soon(request_status)
            await anyio.sleep(0)
            assert sender.send_count == 1
            sender.release_operation.set()

        assert len(results) == 2
        assert sender.send_count == 1

    anyio.run(scenario)


def test_same_external_request_and_operation_keep_the_same_routed_id() -> None:
    async def scenario() -> None:
        broker, sender, _scope = await _selected_operation_broker()
        base_id = RequestId("same-external-request")

        _ = await broker.project_operation(
            "session-a", "subject-a", RepoStatusRequest(), request_id=base_id
        )
        _ = await broker.project_operation(
            "session-a", "subject-a", RepoStatusRequest(), request_id=base_id
        )

        assert sender.commands[-2].request_id == sender.commands[-1].request_id

    anyio.run(scenario)


def test_same_external_request_with_different_operation_gets_a_new_routed_id() -> None:
    async def scenario() -> None:
        broker, sender, _scope = await _selected_operation_broker()
        base_id = RequestId("same-external-request")

        _ = await broker.project_operation(
            "session-a", "subject-a", RepoStatusRequest(), request_id=base_id
        )
        _ = await broker.project_operation(
            "session-a", "subject-a", RepoDiffRequest(), request_id=base_id
        )

        assert sender.commands[-2].request_id != sender.commands[-1].request_id

    anyio.run(scenario)


def test_same_external_request_after_reselect_gets_a_new_routed_id() -> None:
    async def scenario() -> None:
        broker, sender, scope = await _selected_operation_broker()
        base_id = RequestId("same-external-request")

        _ = await broker.project_operation(
            "session-a", "subject-a", RepoStatusRequest(), request_id=base_id
        )
        await broker.select("session-a", "subject-a", scope)
        _ = await broker.project_operation(
            "session-a", "subject-a", RepoStatusRequest(), request_id=base_id
        )

        assert sender.commands[-2].request_id != sender.commands[-1].request_id

    anyio.run(scenario)


async def _selected_operation_broker() -> tuple[BindingBroker, OperationSender, str]:
    broker = BindingBroker()
    sender = OperationSender(broker)
    await broker.attach(DeviceId("device-a"), sender)
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
    await broker.select("session-a", "subject-a", scope)
    return broker, sender, scope
