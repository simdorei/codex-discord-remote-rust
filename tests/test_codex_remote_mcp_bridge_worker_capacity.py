from __future__ import annotations

import threading
import time

import pytest

from codex_remote_mcp_bridge_workers import BridgeCommandWorkers
from codex_remote_mcp_dispatch import LocalProjectDispatcher
from simdorei_mcp_common.messages import (
    BridgeResult,
    OperationErrorResult,
    ProjectInfoCommand,
    ProjectInfoOutput,
    ProjectInfoResult,
    RequestId,
)


def test_worker_capacity_returns_an_explicit_busy_result(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    dispatcher = LocalProjectDispatcher()
    started = threading.Event()
    release = threading.Event()

    def delayed_execute(
        self: LocalProjectDispatcher,
        command: ProjectInfoCommand,
        *,
        connection_generation: int | None = None,
        cancel_event: threading.Event | None = None,
    ) -> BridgeResult:
        _ = self, connection_generation, cancel_event
        started.set()
        assert release.wait(timeout=5)
        return ProjectInfoResult(
            request_id=command.request_id,
            output=ProjectInfoOutput(root="C:/qa", thread_id=command.thread_id),
        )

    monkeypatch.setattr(LocalProjectDispatcher, "execute", delayed_execute)
    workers = BridgeCommandWorkers(
        dispatcher,
        log=lambda _: None,
        max_workers=1,
        max_pending=1,
    )
    generation = workers.begin_connection()

    try:
        first = workers.submit(generation, _command("first"))
        assert first is None
        assert started.wait(timeout=2)

        rejected = workers.submit(generation, _command("second"))
        assert isinstance(rejected, OperationErrorResult)
        assert rejected.error_code == "bridge_busy"
    finally:
        release.set()
        workers.close()


def test_close_does_not_wait_forever_for_a_running_command(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    dispatcher = LocalProjectDispatcher()
    started = threading.Event()
    release = threading.Event()

    def delayed_execute(
        self: LocalProjectDispatcher,
        command: ProjectInfoCommand,
        *,
        connection_generation: int | None = None,
        cancel_event: threading.Event | None = None,
    ) -> BridgeResult:
        _ = self, command, connection_generation
        started.set()
        assert cancel_event is not None
        assert cancel_event.wait(timeout=5)
        release.set()
        return ProjectInfoResult(
            request_id=command.request_id,
            output=ProjectInfoOutput(root="C:/qa", thread_id=command.thread_id),
        )

    monkeypatch.setattr(LocalProjectDispatcher, "execute", delayed_execute)
    workers = BridgeCommandWorkers(dispatcher, log=lambda _: None, max_workers=1)
    _ = workers.submit(workers.begin_connection(), _command("running"))
    assert started.wait(timeout=2)

    started_at = time.monotonic()
    try:
        workers.close()
        assert time.monotonic() - started_at < 0.5
    finally:
        release.set()


def test_submit_after_close_returns_an_explicit_error() -> None:
    workers = BridgeCommandWorkers(
        LocalProjectDispatcher(),
        log=lambda _: None,
        max_workers=1,
    )
    generation = workers.begin_connection()
    workers.close()

    rejected = workers.submit(generation, _command("after-close"))

    assert isinstance(rejected, OperationErrorResult)
    assert rejected.error_code == "bridge_closing"


def _command(request_id: str) -> ProjectInfoCommand:
    return ProjectInfoCommand(
        request_id=RequestId(request_id),
        thread_id="thread-a",
        computer_session_id="computer-session-a",
    )
