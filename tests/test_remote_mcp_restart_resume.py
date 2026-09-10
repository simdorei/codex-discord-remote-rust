from __future__ import annotations

from datetime import UTC, datetime, timedelta
from typing import assert_never, final, override

import anyio
import pytest

from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.broker_errors import BindingCodeError
from remote_mcp_server.simdorei_mcp.broker_models import BridgeSender
from simdorei_mcp_common.messages import (
    DeviceId,
    GatewayCommand,
    ListFilesCommand,
    ProjectInfoCommand,
    ProjectInfoOutput,
    ProjectInfoResult,
    ProjectOperationCommand,
    ProjectSessionCommand,
    ProjectSessionResult,
    ProjectUpsert,
    ReadFileCommand,
    WriteFileCommand,
)


@final
class _RespondingSender(BridgeSender):
    def __init__(
        self,
        broker: BindingBroker,
        device_id: DeviceId | None = None,
    ) -> None:
        self._broker = broker
        self._device_id: DeviceId = (
            device_id if device_id is not None else DeviceId("device-a")
        )
        self.session_commands: list[ProjectSessionCommand] = []

    @override
    async def send(self, command: GatewayCommand) -> None:
        match command:
            case ProjectSessionCommand():
                self.session_commands.append(command)
                result = ProjectSessionResult(request_id=command.request_id)
            case ProjectInfoCommand():
                result = ProjectInfoResult(
                    request_id=command.request_id,
                    output=ProjectInfoOutput(
                        root="C:/work/project-a",
                        thread_id=command.thread_id,
                    ),
                )
            case (
                ListFilesCommand()
                | ReadFileCommand()
                | WriteFileCommand()
                | ProjectOperationCommand()
            ):
                raise AssertionError(f"unexpected command: {command.type}")
            case unreachable:
                assert_never(unreachable)
        await self._broker.complete(self._device_id, self, result)

    @override
    async def close(self) -> None:
        return None


def _project() -> ProjectUpsert:
    return ProjectUpsert(
        project_scope="codex-pro-project-a",
        binding_id="binding-generation-project-a",
        thread_id="thread-a",
        project_name="project-a",
        expires_at=datetime.now(UTC) + timedelta(minutes=10),
    )


def test_exact_binding_reconnect_restores_chat_session_without_reselection() -> None:
    async def scenario() -> None:
        broker = BindingBroker()
        previous = _RespondingSender(broker)
        project = _project()
        _ = await broker.attach(DeviceId("device-a"), previous)
        await broker.upsert(DeviceId("device-a"), previous, project)
        selected = await broker.select(
            "session-a",
            "subject-a",
            project.project_scope,
        )
        previous_session = previous.session_commands[-1]

        await broker.detach(DeviceId("device-a"), previous)
        assert broker.session_route_count == 0

        replacement = _RespondingSender(broker)
        _ = await broker.attach(DeviceId("device-a"), replacement)
        await broker.upsert(DeviceId("device-a"), replacement, project)
        resumed = await broker.resume_project(
            DeviceId("device-a"),
            replacement,
            project,
        )

        assert resumed
        assert broker.session_route_count == 1
        assert replacement.session_commands[-1].computer_session_generation > (
            previous_session.computer_session_generation
        )
        assert replacement.session_commands[-1].computer_session_id != (
            previous_session.computer_session_id
        )
        output = await broker.project_info("session-a", "subject-a")
        assert output.thread_id == selected.thread_id

    anyio.run(scenario)


def test_changed_binding_cannot_resume_detached_chat_session() -> None:
    async def scenario() -> None:
        broker = BindingBroker()
        previous = _RespondingSender(broker)
        project = _project()
        _ = await broker.attach(DeviceId("device-a"), previous)
        await broker.upsert(DeviceId("device-a"), previous, project)
        _ = await broker.select("session-a", "subject-a", project.project_scope)
        await broker.detach(DeviceId("device-a"), previous)

        replacement = _RespondingSender(broker)
        changed = project.model_copy(update={"binding_id": "different-binding"})
        _ = await broker.attach(DeviceId("device-a"), replacement)
        await broker.upsert(DeviceId("device-a"), replacement, changed)

        assert not await broker.resume_project(
            DeviceId("device-a"),
            replacement,
            changed,
        )
        assert broker.session_route_count == 0
        assert broker.dormant_route_count == 0

    anyio.run(scenario)


def test_detached_chat_session_expires_before_late_bridge_reconnect() -> None:
    async def scenario() -> None:
        clock = [datetime.now(UTC)]
        broker = BindingBroker(now=lambda: clock[0])
        previous = _RespondingSender(broker)
        project = _project()
        _ = await broker.attach(DeviceId("device-a"), previous)
        await broker.upsert(DeviceId("device-a"), previous, project)
        _ = await broker.select("session-a", "subject-a", project.project_scope)
        await broker.detach(DeviceId("device-a"), previous)
        clock[0] += timedelta(minutes=3)

        replacement = _RespondingSender(broker)
        _ = await broker.attach(DeviceId("device-a"), replacement)
        await broker.upsert(DeviceId("device-a"), replacement, project)

        assert not await broker.resume_project(
            DeviceId("device-a"),
            replacement,
            project,
        )
        assert broker.session_route_count == 0

    anyio.run(scenario)


def test_one_device_restart_preserves_other_device_and_dormant_scope_owner() -> None:
    async def scenario() -> None:
        broker = BindingBroker()
        sender_a = _RespondingSender(broker, DeviceId("device-a"))
        sender_b = _RespondingSender(broker, DeviceId("device-b"))
        project_a = _project()
        project_b = project_a.model_copy(
            update={
                "project_scope": "codex-pro-project-b",
                "binding_id": "binding-generation-project-b",
                "thread_id": "thread-b",
                "project_name": "project-b",
            }
        )
        _ = await broker.attach(DeviceId("device-a"), sender_a)
        _ = await broker.attach(DeviceId("device-b"), sender_b)
        await broker.upsert(DeviceId("device-a"), sender_a, project_a)
        await broker.upsert(DeviceId("device-b"), sender_b, project_b)
        _ = await broker.select("session-a", "subject-a", project_a.project_scope)
        _ = await broker.select("session-b", "subject-a", project_b.project_scope)

        await broker.detach(DeviceId("device-a"), sender_a)

        output_b = await broker.project_info("session-b", "subject-a")
        assert output_b.thread_id == "thread-b"
        with pytest.raises(BindingCodeError, match="owned by another device"):
            await broker.upsert(
                DeviceId("device-b"),
                sender_b,
                project_a.model_copy(update={"thread_id": "thread-b"}),
            )

        replacement_a = _RespondingSender(broker, DeviceId("device-a"))
        _ = await broker.attach(DeviceId("device-a"), replacement_a)
        await broker.upsert(DeviceId("device-a"), replacement_a, project_a)
        assert await broker.resume_project(
            DeviceId("device-a"),
            replacement_a,
            project_a,
        )
        assert (await broker.project_info("session-a", "subject-a")).thread_id == (
            "thread-a"
        )
        assert (await broker.project_info("session-b", "subject-a")).thread_id == (
            "thread-b"
        )

    anyio.run(scenario)
