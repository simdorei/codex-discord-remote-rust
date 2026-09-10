from __future__ import annotations

from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import assert_never

import anyio

from codex_remote_mcp_dispatch import LocalProjectDispatcher
from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.broker_errors import ActiveBindingMissingError
from remote_mcp_server.simdorei_mcp.broker_models import BridgeSender
from simdorei_mcp_common.messages import (
    DeviceId,
    GatewayCommand,
    ListFilesCommand,
    ProjectInfoCommand,
    ProjectOperationCommand,
    ProjectSessionCommand,
    ProjectSessionResult,
    ProjectUpsert,
    ReadFileCommand,
    RequestId,
    WriteFileCommand,
)
from simdorei_mcp_common.operation_requests import ComputerScreenshotRequest
from tests.remote_mcp_computer_fakes import (
    FakeComputerPlatform,
    computer_window,
    make_controller,
)


class DelayedOldSessionSender(BridgeSender):
    def __init__(
        self,
        broker: BindingBroker,
        dispatcher: LocalProjectDispatcher,
    ) -> None:
        self._broker = broker
        self._dispatcher = dispatcher
        self.old_command_started = anyio.Event()
        self.release_old_command = anyio.Event()
        self.old_generation = ""
        self.session_generations: list[int] = []
        self.delayed_result_type = ""

    async def send(self, command: GatewayCommand) -> None:
        match command:
            case ProjectSessionCommand():
                self.session_generations.append(command.computer_session_generation)
                if not self.old_generation:
                    self.old_generation = command.computer_session_id
                result = self._dispatcher.execute(command)
            case ProjectOperationCommand():
                assert command.computer_session_id == self.old_generation
                self.old_command_started.set()
                await self.release_old_command.wait()
                result = self._dispatcher.execute(command)
                self.delayed_result_type = result.type
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


def _project(scope: str) -> ProjectUpsert:
    return ProjectUpsert(
        project_scope=scope,
        binding_id="binding-generation-project-a",
        thread_id="thread-a",
        project_name="project-a",
        expires_at=datetime.now(UTC) + timedelta(minutes=10),
    )


def test_replacement_barrier_rejects_a_delayed_old_generation(
    tmp_path: Path,
) -> None:
    async def scenario() -> None:
        platform = FakeComputerPlatform(computer_window())
        dispatcher = LocalProjectDispatcher(
            computer_factory=lambda: make_controller(platform),
        )
        dispatcher.upsert(
            "thread-a",
            tmp_path,
            datetime.now(UTC) + timedelta(minutes=10),
        )
        broker = BindingBroker()
        sender = DelayedOldSessionSender(broker, dispatcher)
        await broker.attach(DeviceId("device-a"), sender)
        scope = "codex-pro-project-a"
        await broker.upsert(DeviceId("device-a"), sender, _project(scope))
        await broker.select("session-a", "subject-a", scope)

        old_failure: list[BaseException] = []

        async def old_screenshot() -> None:
            try:
                _ = await broker.project_operation(
                    "session-a",
                    "subject-a",
                    ComputerScreenshotRequest(window_id=42),
                )
            except ActiveBindingMissingError as exc:
                old_failure.append(exc)

        async with anyio.create_task_group() as tasks:
            tasks.start_soon(old_screenshot)
            await sender.old_command_started.wait()
            await broker.select("session-b", "subject-a", scope)
            sender.release_old_command.set()

        assert len(old_failure) == 1
        assert isinstance(old_failure[0], ActiveBindingMissingError)
        assert sender.delayed_result_type == "operation_error"
        assert sender.session_generations == [1, 2]

    anyio.run(scenario)


def test_local_dispatcher_requires_an_acknowledged_session_generation(
    tmp_path: Path,
) -> None:
    dispatcher = LocalProjectDispatcher()
    dispatcher.upsert(
        "thread-a",
        tmp_path,
        datetime.now(UTC) + timedelta(minutes=10),
    )

    activated = dispatcher.execute(
        ProjectSessionCommand(
            request_id=RequestId("activate-a"),
            thread_id="thread-a",
            computer_session_id="computer-session-a",
            computer_session_generation=1,
        )
    )
    replaced = dispatcher.execute(
        ProjectSessionCommand(
            request_id=RequestId("activate-b"),
            thread_id="thread-a",
            computer_session_id="computer-session-b",
            computer_session_generation=2,
        )
    )
    stale = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("stale-a"),
            thread_id="thread-a",
            computer_session_id="computer-session-a",
            operation=ComputerScreenshotRequest(window_id=42),
        )
    )

    assert isinstance(activated, ProjectSessionResult)
    assert isinstance(replaced, ProjectSessionResult)
    assert stale.type == "operation_error"
    assert stale.error_code == "computer_control"
