from __future__ import annotations

from datetime import UTC, datetime, timedelta

import anyio
import pytest

from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.broker_errors import (
    ActiveBindingMissingError,
    BindingCodeError,
)
from simdorei_mcp_common.messages import DeviceId, ProjectUpsert
from tests.test_remote_mcp_broker import (
    CancellableProjectInfoSender,
    ProjectInfoSender,
)


def test_expired_route_prune_cancels_an_inflight_request() -> None:
    async def scenario() -> None:
        clock = [datetime(2026, 8, 1, 12, tzinfo=UTC)]
        broker = BindingBroker(now=lambda: clock[0])
        sender = CancellableProjectInfoSender(broker)
        await broker.attach(DeviceId("device-a"), sender)
        scope = "codex-pro-expiring-project"
        await broker.upsert(
            DeviceId("device-a"),
            sender,
            _project(scope, "thread-a", clock[0] + timedelta(minutes=1)),
        )
        _ = await broker.select("session-a", "subject-a", scope)
        failures: list[ActiveBindingMissingError] = []

        async def request_info() -> None:
            try:
                _ = await broker.project_info("session-a", "subject-a")
            except ActiveBindingMissingError as exc:
                failures.append(exc)

        async with anyio.create_task_group() as tasks:
            tasks.start_soon(request_info)
            await sender.project_info_started.wait()
            clock[0] += timedelta(minutes=2)
            with pytest.raises(ActiveBindingMissingError):
                _ = await broker.project_info("session-a", "subject-a")
            assert broker.registered_project_count == 0
            assert broker.session_route_count == 0
            assert broker.thread_session_count == 0
            assert len(broker._computer_session_generations) == 0
            sender.release_project_info.set()

        assert len(failures) == 1
        assert "expired" in str(failures[0]).lower()
        assert broker.pending_call_count == 0

    anyio.run(scenario)


def test_select_prunes_an_expired_project_and_route() -> None:
    async def scenario() -> None:
        clock = [datetime(2026, 8, 1, 12, tzinfo=UTC)]
        broker = BindingBroker(now=lambda: clock[0])
        sender = ProjectInfoSender(broker)
        await broker.attach(DeviceId("device-a"), sender)
        scope = "codex-pro-expired-selection"
        await broker.upsert(
            DeviceId("device-a"),
            sender,
            _project(scope, "thread-a", clock[0] + timedelta(minutes=1)),
        )
        _ = await broker.select("session-a", "subject-a", scope)
        clock[0] += timedelta(minutes=2)

        with pytest.raises(BindingCodeError, match="unavailable or expired"):
            _ = await broker.select("session-b", "subject-a", scope)

        assert broker.registered_project_count == 0
        assert broker.session_route_count == 0
        assert broker.thread_session_count == 0
        assert len(broker._computer_session_generations) == 0

    anyio.run(scenario)


def test_upsert_opportunistically_prunes_other_expired_projects() -> None:
    async def scenario() -> None:
        clock = [datetime(2026, 8, 1, 12, tzinfo=UTC)]
        broker = BindingBroker(now=lambda: clock[0])
        sender = ProjectInfoSender(broker)
        await broker.attach(DeviceId("device-a"), sender)
        await broker.upsert(
            DeviceId("device-a"),
            sender,
            _project(
                "codex-pro-expired-project",
                "thread-a",
                clock[0] + timedelta(minutes=1),
            ),
        )
        clock[0] += timedelta(minutes=2)

        await broker.upsert(
            DeviceId("device-a"),
            sender,
            _project(
                "codex-pro-current-project",
                "thread-b",
                clock[0] + timedelta(minutes=10),
            ),
        )

        assert broker.registered_project_count == 1

    anyio.run(scenario)


def test_same_binding_renewal_extends_the_selected_chat_route() -> None:
    async def scenario() -> None:
        clock = [datetime(2026, 8, 1, 12, tzinfo=UTC)]
        broker = BindingBroker(now=lambda: clock[0])
        sender = ProjectInfoSender(broker)
        _ = await broker.attach(DeviceId("device-a"), sender)
        scope = "codex-pro-renewed-project"
        initial_expiry = clock[0] + timedelta(minutes=1)
        await broker.upsert(
            DeviceId("device-a"),
            sender,
            _project(scope, "thread-a", initial_expiry),
        )
        _ = await broker.select("session-a", "subject-a", scope)

        renewed_expiry = clock[0] + timedelta(minutes=3)
        await broker.upsert(
            DeviceId("device-a"),
            sender,
            _project(scope, "thread-a", renewed_expiry),
        )
        clock[0] = initial_expiry + timedelta(seconds=1)

        result = await broker.project_info("session-a", "subject-a")

        assert result.thread_id == "thread-a"
        assert broker.registered_project_count == 1
        assert broker.session_route_count == 1
        assert broker.thread_session_count == 1

    anyio.run(scenario)


def _project(
    scope: str,
    thread_id: str,
    expires_at: datetime,
) -> ProjectUpsert:
    return ProjectUpsert(
        project_scope=scope,
        binding_id=f"binding-generation-{thread_id}",
        thread_id=thread_id,
        project_name=thread_id,
        expires_at=expires_at,
    )
