from __future__ import annotations

from collections.abc import Callable
from datetime import UTC, datetime, timedelta
from pathlib import Path
import threading
from time import monotonic, sleep

from codex_remote_mcp_bridge import RemoteMcpBridge
from codex_remote_mcp_bridge_config import RemoteMcpBridgeConfig
from codex_remote_mcp_bridge_connection import RemoteMcpBridgeError
from codex_remote_mcp_computer import ComputerController
from codex_remote_mcp_dispatch_state import ProjectDispatchState
from simdorei_mcp_common.messages import (
    ProjectAck,
    ProjectInfoCommand,
    ProjectInfoResult,
    ProjectSessionCommand,
    ProjectUpsert,
    RequestId,
)
from tests.test_codex_remote_mcp_bridge import FakeConnector, FakeSocket


class DelayedReplacementAckSocket(FakeSocket):
    def __init__(self) -> None:
        super().__init__()
        self.new_sent = threading.Event()
        self.stale_renewal_sent = threading.Event()
        self._old_send_count = 0

    def send(self, message: str) -> None:
        if '"type":"hello"' in message:
            super().send(message)
            return
        self.sent.append(message)
        if '"type":"project_upsert"' not in message:
            return
        project = ProjectUpsert.model_validate_json(message)
        if project.project_scope == "codex-pro-project-old":
            self._old_send_count += 1
            if self._old_send_count > 1:
                self.stale_renewal_sent.set()
            self.ack(project)
            return
        self.new_sent.set()

    def ack(self, project: ProjectUpsert) -> None:
        self.inbound.put(
            ProjectAck(
                project_scope=project.project_scope,
                binding_id=project.binding_id,
            ).model_dump_json()
        )


def test_connected_bridge_renews_binding_without_replacing_session(
    tmp_path: Path,
) -> None:
    now = [datetime.now(UTC)]
    socket = FakeSocket()
    bridge = RemoteMcpBridge(
        _config(),
        connector=FakeConnector(socket),
        log=lambda _: None,
        now=lambda: now[0],
    )

    try:
        _ = bridge.register_project(
            "thread-1",
            "codex-pro-renewed-project",
            tmp_path,
        )
        initial = next(
            ProjectUpsert.model_validate_json(raw)
            for raw in socket.sent
            if '"type":"project_upsert"' in raw
        )
        session_id = "project-session-generation-1"
        socket.inbound.put(
            ProjectSessionCommand(
                request_id=RequestId("activate-renewed-session"),
                thread_id="thread-1",
                computer_session_id=session_id,
                computer_session_generation=1,
            ).model_dump_json()
        )
        _ = _wait_for_sent(
            socket.sent,
            lambda raw: '"type":"project_session_result"' in raw,
        )

        now[0] += timedelta(seconds=6)
        renewed_raw = _wait_for_sent(
            socket.sent,
            lambda raw: (
                '"type":"project_upsert"' in raw
                and ProjectUpsert.model_validate_json(raw).expires_at
                > initial.expires_at
            ),
        )
        renewed = ProjectUpsert.model_validate_json(renewed_raw)
        socket.inbound.put(
            ProjectInfoCommand(
                request_id=RequestId("info-after-renewal"),
                thread_id="thread-1",
                computer_session_id=session_id,
            ).model_dump_json()
        )
        result_raw = _wait_for_sent(
            socket.sent,
            lambda raw: '"request_id":"info-after-renewal"' in raw,
        )
    finally:
        bridge.close()

    assert renewed.binding_id == initial.binding_id
    assert renewed.expires_at == now[0] + timedelta(seconds=10)
    assert isinstance(
        ProjectInfoResult.model_validate_json(result_raw), ProjectInfoResult
    )


def test_pending_replacement_prevents_stale_binding_renewal(tmp_path: Path) -> None:
    now = [datetime.now(UTC)]
    socket = DelayedReplacementAckSocket()
    bridge = RemoteMcpBridge(
        _config(),
        connector=FakeConnector(socket),
        log=lambda _: None,
        now=lambda: now[0],
    )
    errors: list[RemoteMcpBridgeError] = []

    def register_replacement() -> None:
        try:
            _ = bridge.register_project(
                "thread-1",
                "codex-pro-project-new",
                tmp_path,
            )
        except RemoteMcpBridgeError as exc:
            errors.append(exc)

    worker = threading.Thread(target=register_replacement)
    try:
        _ = bridge.register_project(
            "thread-1",
            "codex-pro-project-old",
            tmp_path,
        )
        worker.start()
        assert socket.new_sent.wait(timeout=2)
        now[0] += timedelta(seconds=6)

        assert not socket.stale_renewal_sent.wait(timeout=0.6)

        replacement = next(
            ProjectUpsert.model_validate_json(raw)
            for raw in socket.sent
            if '"type":"project_upsert"' in raw
            and ProjectUpsert.model_validate_json(raw).project_scope
            == "codex-pro-project-new"
        )
        socket.ack(replacement)
        worker.join(timeout=2)
    finally:
        bridge.close()

    assert not worker.is_alive()
    assert errors == []


def test_lease_renewal_does_not_wait_for_running_project_command(
    tmp_path: Path,
) -> None:
    def unexpected_computer() -> ComputerController:
        raise AssertionError("computer controller should not be created")

    state = ProjectDispatchState(unexpected_computer)
    initial_expiry = datetime.now(UTC) + timedelta(seconds=10)
    renewed_expiry = initial_expiry + timedelta(seconds=10)
    state.upsert("thread-1", tmp_path, initial_expiry)
    project = state.binding("thread-1")
    assert project is not None
    completed = threading.Event()

    def renew() -> None:
        state.renew("thread-1", renewed_expiry)
        completed.set()

    project.execution_lock.acquire()
    worker = threading.Thread(target=renew)
    try:
        worker.start()
        assert completed.wait(timeout=0.2)
    finally:
        project.execution_lock.release()
        worker.join(timeout=2)

    assert not worker.is_alive()
    assert project.expires_at == renewed_expiry


def _config() -> RemoteMcpBridgeConfig:
    return RemoteMcpBridgeConfig(
        bridge_url="wss://example.test/bridge",
        device_id="device-1",
        device_token="secret-token",
        binding_ttl_seconds=10,
        binding_ack_timeout_seconds=2,
        reconnect_delay_seconds=0.01,
    )


def _wait_for_sent(messages: list[str], predicate: Callable[[str], bool]) -> str:
    deadline = monotonic() + 2
    while monotonic() < deadline:
        for message in messages:
            if predicate(message):
                return message
        sleep(0.01)
    raise AssertionError("expected bridge message was not sent")
