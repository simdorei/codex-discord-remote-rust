from __future__ import annotations

from datetime import UTC, datetime, timedelta
import threading

import pytest

from codex_remote_mcp_idempotency import IdempotentResultCache
from simdorei_mcp_common.messages import (
    ProjectOperationCommand,
    ProjectOperationResult,
    RequestId,
)
from simdorei_mcp_common.operation_outputs import RepoStatusOutput
from simdorei_mcp_common.operation_requests import FileApplyPatchRequest, FileChange
from simdorei_mcp_common.request_deadlines import (
    RequestBudget,
    RequestDeadlineExpired,
)


def test_identical_request_reuses_the_cached_result() -> None:
    cache = IdempotentResultCache()
    first = _command()
    repeated = _command()
    result = ProjectOperationResult(
        request_id=first.request_id,
        output=RepoStatusOutput(
            branch="main",
            dirty_files=(),
            staged_files=(),
            remotes=(),
            upstream=None,
            ahead=0,
            behind=0,
        ),
    )
    executions = 0

    def execute_once() -> ProjectOperationResult:
        nonlocal executions
        executions += 1
        return result

    assert cache.execute_once(first, execute_once, budget=_live_budget()) is result
    assert cache.execute_once(repeated, execute_once, budget=_live_budget()) is result
    assert executions == 1


def test_duplicate_wait_obeys_its_own_deadline() -> None:
    cache = IdempotentResultCache()
    command = _command()
    started = threading.Event()
    release = threading.Event()

    def blocked_execute() -> ProjectOperationResult:
        started.set()
        assert release.wait(timeout=5)
        return ProjectOperationResult(
            request_id=command.request_id,
            output=RepoStatusOutput(
                branch="main",
                dirty_files=(),
                staged_files=(),
                remotes=(),
                upstream=None,
                ahead=0,
                behind=0,
            ),
        )

    worker = threading.Thread(
        target=lambda: cache.execute_once(
            command,
            blocked_execute,
            budget=_live_budget(),
        )
    )
    worker.start()
    assert started.wait(timeout=2)
    expired = RequestBudget(_deadline_monotonic=0.0, _clock=lambda: 1.0)

    with pytest.raises(RequestDeadlineExpired):
        _ = cache.execute_once(command, blocked_execute, budget=expired)

    release.set()
    worker.join(timeout=2)
    assert not worker.is_alive()


def _command() -> ProjectOperationCommand:
    return ProjectOperationCommand(
        request_id=RequestId("routed-request-id"),
        thread_id="thread-a",
        computer_session_id="computer-session-a",
        operation=FileApplyPatchRequest(
            changes=(FileChange(action="create", path="a.txt", content="a"),),
        ),
    )


def _live_budget() -> RequestBudget:
    return RequestBudget.from_deadline(datetime.now(UTC) + timedelta(minutes=1))
