from __future__ import annotations

from datetime import UTC, datetime, timedelta
from typing import assert_never, override

import anyio
import pytest

from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.broker_errors import SessionProjectConflictError
from remote_mcp_server.simdorei_mcp.broker_models import BridgeSender
from simdorei_mcp_common.messages import (
    DeviceId,
    DeviceSessionCommand,
    GatewayCommand,
    ProjectSessionCommand,
    ProjectSessionResult,
    ProjectUpsert,
)


class SessionActivationSender(BridgeSender):
    def __init__(self, broker: BindingBroker, device_id: DeviceId) -> None:
        self._broker = broker
        self._device_id = device_id

    @override
    async def send(self, command: GatewayCommand) -> None:
        match command:
            case DeviceSessionCommand() | ProjectSessionCommand():
                result = ProjectSessionResult(request_id=command.request_id)
            case unreachable:
                assert_never(unreachable)
        await self._broker.complete(self._device_id, self, result)

    @override
    async def close(self) -> None:
        return None


def test_chat_can_return_from_device_mode_to_the_existing_project_mode() -> None:
    async def scenario() -> None:
        broker = BindingBroker()
        device_id = DeviceId("desktop-pc")
        sender = SessionActivationSender(broker, device_id)
        await broker.attach(device_id, sender)
        _ = await broker.select_device(
            "chat-session-a",
            "subject-a",
            device_id,
            working_directory="D:/ERP",
        )
        project_scope = "codex-pro-project-a"
        await broker.upsert(
            device_id,
            sender,
            ProjectUpsert(
                project_scope=project_scope,
                binding_id="binding-generation-project-a",
                thread_id="thread-a",
                project_name="project-a",
                expires_at=datetime.now(UTC) + timedelta(minutes=10),
            ),
        )

        output = await broker.select(
            "chat-session-a",
            "subject-a",
            project_scope,
        )

        assert output.thread_id == "thread-a"
        assert output.project_name == "project-a"

    anyio.run(scenario)


def test_chat_can_switch_from_project_mode_to_device_mode() -> None:
    async def scenario() -> None:
        broker = BindingBroker()
        device_id = DeviceId("desktop-pc")
        sender = SessionActivationSender(broker, device_id)
        await broker.attach(device_id, sender)
        project_scope = "codex-pro-project-before-device"
        await broker.upsert(
            device_id,
            sender,
            ProjectUpsert(
                project_scope=project_scope,
                binding_id="binding-before-device",
                thread_id="thread-before-device",
                project_name="project-before-device",
                expires_at=datetime.now(UTC) + timedelta(minutes=10),
            ),
        )
        _ = await broker.select("chat-session-a", "subject-a", project_scope)

        output = await broker.select_device(
            "chat-session-a",
            "subject-a",
            device_id,
            working_directory="C:/Work",
        )

        assert output.device_id == device_id
        assert output.working_directory == "C:/Work"

    anyio.run(scenario)


def test_device_selection_cannot_replace_another_oauth_subject() -> None:
    async def scenario() -> None:
        broker = BindingBroker()
        device_id = DeviceId("desktop-pc")
        sender = SessionActivationSender(broker, device_id)
        await broker.attach(device_id, sender)
        _ = await broker.select_device(
            "shared-chat-session",
            "subject-owner",
            device_id,
            working_directory="C:/Owner",
        )

        with pytest.raises(SessionProjectConflictError):
            _ = await broker.select_device(
                "shared-chat-session",
                "subject-attacker",
                device_id,
                working_directory="C:/Attacker",
            )

    anyio.run(scenario)
