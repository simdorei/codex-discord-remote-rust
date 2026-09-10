from __future__ import annotations

from codex_remote_mcp_dispatch import LocalProjectDispatcher
from simdorei_mcp_common.messages import (
    ProjectSessionCommand,
    ProjectSessionResult,
    RequestId,
)

TEST_PROJECT_SESSION_ID = "test-project-session-generation"


def activate_test_session(
    dispatcher: LocalProjectDispatcher,
    thread_id: str = "thread-a",
) -> None:
    result = dispatcher.execute(
        ProjectSessionCommand(
            request_id=RequestId(f"activate-{thread_id}"),
            thread_id=thread_id,
            computer_session_id=TEST_PROJECT_SESSION_ID,
            computer_session_generation=1,
        )
    )
    assert isinstance(result, ProjectSessionResult)
