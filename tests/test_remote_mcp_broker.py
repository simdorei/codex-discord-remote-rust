"""Binding broker behavior tests. (# noqa: SIZE_OK)"""

from __future__ import annotations

from datetime import UTC, datetime, timedelta
from typing import assert_never, override

import anyio
import pytest

from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.broker_errors import (
    ActiveBindingMissingError,
    BindingCodeError,
    BridgeTimeoutError,
    BridgeUnavailableError,
    BrokerError,
)
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


class ProjectInfoSender(BridgeSender):
    """In-memory bridge that completes project-info commands."""

    def __init__(
        self,
        broker: BindingBroker,
        device_id: DeviceId | None = None,
    ) -> None:
        self._broker: BindingBroker = broker
        self._device_id: DeviceId = (
            device_id if device_id is not None else DeviceId("device-a")
        )
        self.commands: list[ProjectInfoCommand] = []

    @override
    async def send(self, command: GatewayCommand) -> None:
        # Given
        match command:
            case ProjectInfoCommand():
                self.commands.append(command)
                result = ProjectInfoResult(
                    request_id=command.request_id,
                    output=ProjectInfoOutput(
                        root="C:/work/project-a",
                        thread_id=command.thread_id,
                    ),
                )
            case ProjectSessionCommand():
                result = ProjectSessionResult(request_id=command.request_id)
            case (
                ListFilesCommand()
                | ReadFileCommand()
                | WriteFileCommand()
                | ProjectOperationCommand()
            ):
                raise AssertionError(f"unexpected command: {command.type}")
            case _:
                assert_never(command)

        # When
        await self._broker.complete(self._device_id, self, result)

    @override
    async def close(self) -> None:
        return None


class CancellableProjectInfoSender(ProjectInfoSender):
    def __init__(self, broker: BindingBroker) -> None:
        super().__init__(broker)
        self.project_info_started: anyio.Event = anyio.Event()
        self.release_project_info: anyio.Event = anyio.Event()
        self.close_called: anyio.Event = anyio.Event()

    @override
    async def send(self, command: GatewayCommand) -> None:
        if isinstance(command, ProjectInfoCommand):
            self.project_info_started.set()
            await self.release_project_info.wait()
            return
        await super().send(command)

    @override
    async def close(self) -> None:
        self.close_called.set()


class ReplacingProjectInfoSender(CancellableProjectInfoSender):
    def __init__(
        self,
        broker: BindingBroker,
        replacement: BridgeSender,
    ) -> None:
        super().__init__(broker)
        self._replacement = replacement

    @override
    async def send(self, command: GatewayCommand) -> None:
        if not isinstance(command, ProjectInfoCommand):
            await super().send(command)
            return
        self.project_info_started.set()
        try:
            await self.release_project_info.wait()
        finally:
            with anyio.CancelScope(shield=True):
                _ = await self._broker.attach(
                    DeviceId("device-a"),
                    self._replacement,
                )


def _project(scope: str, thread_id: str = "thread-a") -> ProjectUpsert:
    return ProjectUpsert(
        project_scope=scope,
        binding_id="binding-generation-project-a",
        thread_id=thread_id,
        project_name="project-a",
        expires_at=datetime.now(UTC) + timedelta(minutes=10),
    )


def test_new_chat_session_revokes_previous_session_for_thread() -> None:
    async def scenario() -> None:
        # Given
        broker = BindingBroker()
        sender = ProjectInfoSender(broker)
        await broker.attach(DeviceId("device-a"), sender)
        project_scope = "codex-pro-project-a"
        await broker.upsert(DeviceId("device-a"), sender, _project(project_scope))
        _ = await broker.select("session-a", "subject-a", project_scope)

        # When
        _ = await broker.select("session-b", "subject-a", project_scope)

        # Then
        with pytest.raises(ActiveBindingMissingError):
            _ = await broker.project_info("session-a", "subject-a")
        output = await broker.project_info("session-b", "subject-a")
        assert output.thread_id == "thread-a"

    anyio.run(scenario)


def test_existing_chat_session_cannot_switch_to_another_codex_thread() -> None:
    async def scenario() -> None:
        # Given
        broker = BindingBroker()
        sender = ProjectInfoSender(broker)
        await broker.attach(DeviceId("device-a"), sender)
        first_scope = "codex-pro-project-a"
        second_scope = "codex-pro-project-b"
        await broker.upsert(
            DeviceId("device-a"), sender, _project(first_scope, "thread-a")
        )
        _ = await broker.select("session-a", "subject-a", first_scope)
        await broker.upsert(
            DeviceId("device-a"), sender, _project(second_scope, "thread-b")
        )

        # When / Then
        with pytest.raises(BrokerError, match="different Codex thread"):
            _ = await broker.select("session-a", "subject-a", second_scope)
        output = await broker.project_info("session-a", "subject-a")
        assert output.thread_id == "thread-a"
        rebound = await broker.select("session-b", "subject-a", second_scope)
        assert rebound.thread_id == "thread-b"

    anyio.run(scenario)


def test_two_devices_cannot_claim_the_same_project_scope() -> None:
    async def scenario() -> None:
        broker = BindingBroker()
        sender_a = ProjectInfoSender(broker, DeviceId("device-a"))
        sender_b = ProjectInfoSender(broker, DeviceId("device-b"))
        _ = await broker.attach(DeviceId("device-a"), sender_a)
        _ = await broker.attach(DeviceId("device-b"), sender_b)
        project_scope = "codex-pro-shared-scope"
        await broker.upsert(DeviceId("device-a"), sender_a, _project(project_scope))

        with pytest.raises(BindingCodeError, match="owned by another device"):
            await broker.upsert(
                DeviceId("device-b"),
                sender_b,
                _project(project_scope, "thread-b"),
            )

        selected = await broker.select("session-a", "subject-a", project_scope)
        assert selected.thread_id == "thread-a"
        output = await broker.project_info("session-a", "subject-a")
        assert output.thread_id == "thread-a"
        assert await broker.connected_device_count() == 2

    anyio.run(scenario)


def test_project_scope_race_has_one_owner_without_partial_mutation() -> None:
    async def scenario() -> None:
        broker = BindingBroker()
        sender_a = ProjectInfoSender(broker, DeviceId("device-a"))
        sender_b = ProjectInfoSender(broker, DeviceId("device-b"))
        _ = await broker.attach(DeviceId("device-a"), sender_a)
        _ = await broker.attach(DeviceId("device-b"), sender_b)
        outcomes: list[str] = []

        async def register(
            device_id: DeviceId,
            sender: ProjectInfoSender,
            thread_id: str,
        ) -> None:
            try:
                await broker.upsert(
                    device_id,
                    sender,
                    _project("codex-pro-raced-scope", thread_id),
                )
            except BindingCodeError:
                outcomes.append("conflict")
            else:
                outcomes.append("owner")

        async with anyio.create_task_group() as tasks:
            tasks.start_soon(register, DeviceId("device-a"), sender_a, "thread-a")
            tasks.start_soon(register, DeviceId("device-b"), sender_b, "thread-b")

        assert sorted(outcomes) == ["conflict", "owner"]
        selected = await broker.select(
            "session-race",
            "subject-a",
            "codex-pro-raced-scope",
        )
        assert selected.thread_id in {"thread-a", "thread-b"}

    anyio.run(scenario)


def test_device_disconnect_revokes_bound_sessions() -> None:
    async def scenario() -> None:
        # Given
        broker = BindingBroker()
        sender = ProjectInfoSender(broker)
        await broker.attach(DeviceId("device-a"), sender)
        project_scope = "codex-pro-project-a"
        await broker.upsert(DeviceId("device-a"), sender, _project(project_scope))
        _ = await broker.select("session-a", "subject-a", project_scope)

        # When
        await broker.detach(DeviceId("device-a"), sender)

        # Then
        assert len(broker._computer_session_generations) == 0
        with pytest.raises(ActiveBindingMissingError):
            _ = await broker.project_info("session-a", "subject-a")

    anyio.run(scenario)


def test_project_info_is_forwarded_to_bound_device() -> None:
    async def scenario() -> None:
        # Given
        broker = BindingBroker()
        sender = ProjectInfoSender(broker)
        await broker.attach(DeviceId("device-a"), sender)
        project_scope = "codex-pro-project-a"
        await broker.upsert(DeviceId("device-a"), sender, _project(project_scope))
        _ = await broker.select("session-a", "subject-a", project_scope)

        # When
        output = await broker.project_info("session-a", "subject-a")

        # Then
        assert output.root == "C:/work/project-a"
        assert len(sender.commands) == 1
        assert sender.commands[0].computer_session_id is not None

    anyio.run(scenario)


def test_cancelled_request_is_removed_from_pending_calls() -> None:
    async def scenario() -> None:
        broker = BindingBroker()
        sender = CancellableProjectInfoSender(broker)
        await broker.attach(DeviceId("device-a"), sender)
        project_scope = "codex-pro-project-a"
        await broker.upsert(DeviceId("device-a"), sender, _project(project_scope))
        _ = await broker.select("session-a", "subject-a", project_scope)

        async with anyio.create_task_group() as tasks:
            _ = tasks.start_soon(
                broker.project_info,
                "session-a",
                "subject-a",
            )
            await sender.project_info_started.wait()
            tasks.cancel_scope.cancel()

        assert broker.pending_call_count == 0

    anyio.run(scenario)


def test_blocked_sender_send_times_out_retires_device_and_cleans_pending() -> None:
    async def scenario() -> None:
        broker = BindingBroker(request_timeout_seconds=0.05)
        sender = CancellableProjectInfoSender(broker)
        _ = await broker.attach(DeviceId("device-a"), sender)
        project_scope = "codex-pro-project-a"
        await broker.upsert(DeviceId("device-a"), sender, _project(project_scope))
        _ = await broker.select("session-a", "subject-a", project_scope)

        with anyio.fail_after(0.2):
            with pytest.raises(BridgeTimeoutError, match="local bridge send timed out"):
                _ = await broker.project_info("session-a", "subject-a")

        assert sender.close_called.is_set()
        assert not await broker.is_device_connected(DeviceId("device-a"))
        assert broker.pending_call_count == 0
        with pytest.raises(ActiveBindingMissingError):
            _ = await broker.project_info("session-a", "subject-a")

    anyio.run(scenario)


def test_send_timeout_closes_only_old_sender_after_replacement_attaches() -> None:
    async def scenario() -> None:
        broker = BindingBroker(request_timeout_seconds=0.05)
        replacement = CancellableProjectInfoSender(broker)
        sender = ReplacingProjectInfoSender(broker, replacement)
        _ = await broker.attach(DeviceId("device-a"), sender)
        project_scope = "codex-pro-project-a"
        await broker.upsert(DeviceId("device-a"), sender, _project(project_scope))
        _ = await broker.select("session-a", "subject-a", project_scope)

        with anyio.fail_after(0.2):
            with pytest.raises(BridgeTimeoutError, match="local bridge send timed out"):
                _ = await broker.project_info("session-a", "subject-a")

        assert sender.close_called.is_set()
        assert not replacement.close_called.is_set()
        assert await broker.is_device_connected(DeviceId("device-a"))
        await broker.upsert(
            DeviceId("device-a"),
            replacement,
            _project("codex-pro-project-replacement"),
        )
        assert broker.pending_call_count == 0

    anyio.run(scenario)


def test_fresh_registration_revokes_old_scope_and_session() -> None:
    async def scenario() -> None:
        broker = BindingBroker()
        sender = ProjectInfoSender(broker)
        await broker.attach(DeviceId("device-a"), sender)
        old_scope = "codex-pro-project-old"
        new_scope = "codex-pro-project-new"
        await broker.upsert(DeviceId("device-a"), sender, _project(old_scope))
        _ = await broker.select("session-old", "subject-a", old_scope)

        await broker.upsert(DeviceId("device-a"), sender, _project(new_scope))

        with pytest.raises(ActiveBindingMissingError):
            _ = await broker.project_info("session-old", "subject-a")
        with pytest.raises(BindingCodeError):
            _ = await broker.select("session-replay", "subject-a", old_scope)
        selected = await broker.select("session-new", "subject-a", new_scope)
        assert selected.thread_id == "thread-a"

    anyio.run(scenario)


def test_replaced_bridge_sender_cannot_restore_an_old_scope() -> None:
    async def scenario() -> None:
        broker = BindingBroker()
        stale_sender = ProjectInfoSender(broker)
        current_sender = ProjectInfoSender(broker)
        assert await broker.attach(DeviceId("device-a"), stale_sender) is None
        old_scope = "codex-pro-project-old"
        new_scope = "codex-pro-project-new"
        await broker.upsert(
            DeviceId("device-a"),
            stale_sender,
            _project(old_scope),
        )
        _ = await broker.select("session-old", "subject-a", old_scope)

        assert await broker.attach(DeviceId("device-a"), current_sender) is stale_sender
        await broker.upsert(
            DeviceId("device-a"),
            current_sender,
            _project(new_scope),
        )

        with pytest.raises(BridgeUnavailableError):
            await broker.upsert(
                DeviceId("device-a"),
                stale_sender,
                _project(old_scope),
            )
        with pytest.raises(ActiveBindingMissingError):
            _ = await broker.project_info("session-old", "subject-a")
        with pytest.raises(BindingCodeError):
            _ = await broker.select("session-replay", "subject-a", old_scope)
        selected = await broker.select("session-new", "subject-a", new_scope)
        assert selected.thread_id == "thread-a"

    anyio.run(scenario)
