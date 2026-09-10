from __future__ import annotations

from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import assert_never, override

import anyio

from codex_remote_mcp_computer import ComputerController
from codex_remote_mcp_dispatch import LocalProjectDispatcher
from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.broker_models import BridgeSender
from simdorei_mcp_common.messages import (
    DeviceId,
    DeviceSessionCommand,
    GatewayCommand,
    ProjectInfoCommand,
    ProjectInfoResult,
    ProjectOperationCommand,
    ProjectOperationResult,
    ProjectSessionResult,
    RequestId,
)
from simdorei_mcp_common.operation_outputs import ComputerWindowsOutput
from simdorei_mcp_common.operation_requests import ComputerListWindowsRequest
from tests.remote_mcp_computer_fakes import (
    FakeComputerPlatform,
    computer_window,
    make_controller,
)


class PassiveSender(BridgeSender):
    @override
    async def send(self, command: GatewayCommand) -> None:
        raise AssertionError(f"unexpected command: {command.type}")

    @override
    async def close(self) -> None:
        return None


class DeviceSessionSender(BridgeSender):
    def __init__(self, broker: BindingBroker, device_id: DeviceId) -> None:
        self._broker = broker
        self._device_id = device_id
        self.commands: list[DeviceSessionCommand] = []

    @override
    async def send(self, command: GatewayCommand) -> None:
        match command:
            case DeviceSessionCommand():
                self.commands.append(command)
            case unreachable:
                assert_never(unreachable)
        await self._broker.complete(
            self._device_id,
            self,
            ProjectSessionResult(request_id=command.request_id),
        )

    @override
    async def close(self) -> None:
        return None


def test_connected_devices_are_listed_in_stable_order() -> None:
    async def scenario() -> None:
        # Given
        broker = BindingBroker()
        sender_a = PassiveSender()
        sender_b = PassiveSender()
        await broker.attach(DeviceId("office-pc"), sender_b)
        await broker.attach(DeviceId("desktop-pc"), sender_a)

        # When
        output = await broker.list_devices()

        # Then
        assert tuple(device.device_id for device in output.devices) == (
            DeviceId("desktop-pc"),
            DeviceId("office-pc"),
        )
        assert all(device.online for device in output.devices)

    anyio.run(scenario)

def test_device_selection_binds_the_chat_to_an_arbitrary_working_directory() -> None:
    async def scenario() -> None:
        # Given
        broker = BindingBroker()
        device_id = DeviceId("desktop-pc")
        sender = DeviceSessionSender(broker, device_id)
        await broker.attach(device_id, sender)

        # When
        output = await broker.select_device(
            "chat-session-a",
            "subject-a",
            device_id,
            working_directory="D:/ERP",
        )

        # Then
        assert output.device_id == device_id
        assert output.working_directory == "D:/ERP"
        assert len(sender.commands) == 1
        command = sender.commands[0]
        assert command.type == "device_session"
        assert command.working_directory == "D:/ERP"

    anyio.run(scenario)


def test_device_session_creates_a_local_binding_for_its_working_directory(
    tmp_path: Path,
) -> None:
    # Given
    dispatcher = LocalProjectDispatcher()
    session_id = "device-session-generation-a"
    activated = dispatcher.execute(
        DeviceSessionCommand(
            request_id=RequestId("activate-device-a"),
            thread_id="codex-device-control",
            computer_session_id=session_id,
            computer_session_generation=1,
            working_directory=str(tmp_path),
            expires_at=datetime.now(UTC) + timedelta(minutes=10),
        )
    )

    # When
    result = dispatcher.execute(
        ProjectInfoCommand(
            request_id=RequestId("device-info-a"),
            thread_id="codex-device-control",
            computer_session_id=session_id,
        )
    )

    # Then
    assert isinstance(activated, ProjectSessionResult)
    assert isinstance(result, ProjectInfoResult)
    assert Path(result.output.root) == tmp_path.resolve()


def test_device_session_uses_the_full_device_computer_controller(tmp_path: Path) -> None:
    project_calls: list[bool] = []
    device_calls: list[bool] = []
    platform = FakeComputerPlatform(computer_window())

    def project_factory() -> ComputerController:
        project_calls.append(True)
        return make_controller(platform)

    def device_factory() -> ComputerController:
        device_calls.append(True)
        return make_controller(platform)

    dispatcher = LocalProjectDispatcher(
        computer_factory=project_factory,
        device_computer_factory=device_factory,
    )
    session_id = "device-session-generation-computer"
    activated = dispatcher.execute(
        DeviceSessionCommand(
            request_id=RequestId("activate-device-computer"),
            thread_id="codex-device-control",
            computer_session_id=session_id,
            computer_session_generation=1,
            working_directory=str(tmp_path),
            expires_at=datetime.now(UTC) + timedelta(minutes=10),
        )
    )

    result = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("list-device-windows"),
            thread_id="codex-device-control",
            computer_session_id=session_id,
            operation=ComputerListWindowsRequest(),
        )
    )

    assert isinstance(activated, ProjectSessionResult)
    assert isinstance(result, ProjectOperationResult)
    assert isinstance(result.output, ComputerWindowsOutput)
    assert project_calls == []
    assert device_calls == [True]


def test_selected_device_working_directory_can_change_without_a_project_scope() -> None:
    async def scenario() -> None:
        # Given
        broker = BindingBroker()
        device_id = DeviceId("desktop-pc")
        sender = DeviceSessionSender(broker, device_id)
        await broker.attach(device_id, sender)
        _ = await broker.select_device(
            "chat-session-a",
            "subject-a",
            device_id,
            working_directory="D:/ERP",
        )

        # When
        output = await broker.set_working_directory(
            "chat-session-a",
            "subject-a",
            "C:/Downloads",
        )
        info = await broker.device_info("chat-session-a", "subject-a")

        # Then
        assert output.device_id == device_id
        assert output.working_directory == "C:/Downloads"
        assert info == output
        assert tuple(command.working_directory for command in sender.commands) == (
            "D:/ERP",
            "C:/Downloads",
        )

    anyio.run(scenario)
