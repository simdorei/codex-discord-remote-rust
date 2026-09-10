from __future__ import annotations

import pytest
from pydantic import TypeAdapter, ValidationError

from simdorei_mcp_common.operation_outputs import ProjectOperationOutput
from simdorei_mcp_common.operation_requests import ProjectOperation
from simdorei_mcp_common.terminal_protocol import TerminalExecRequest
from simdorei_mcp_common.terminal_window_protocol import (
    TerminalWindowCloseRequest,
    TerminalWindowEntry,
    TerminalWindowListOutput,
    TerminalWindowListRequest,
    TerminalWindowOpenRequest,
    is_terminal_window_request,
)

WINDOW_ID = "termwin_0123456789abcdef"


def test_terminal_window_requests_round_trip_through_project_protocol() -> None:
    adapter: TypeAdapter[ProjectOperation] = TypeAdapter(ProjectOperation)

    opened = adapter.validate_python(
        {"kind": "terminal_window_open", "shell": "cmd", "cwd": "tools"}
    )
    listed = adapter.validate_python({"kind": "terminal_window_list"})
    closed = adapter.validate_python(
        {"kind": "terminal_window_close", "terminal_window_id": WINDOW_ID}
    )

    assert isinstance(opened, TerminalWindowOpenRequest)
    assert opened.shell == "cmd"
    assert opened.cwd == "tools"
    assert isinstance(listed, TerminalWindowListRequest)
    assert isinstance(closed, TerminalWindowCloseRequest)
    assert closed.terminal_window_id == WINDOW_ID


def test_terminal_window_output_round_trips_through_project_protocol() -> None:
    output = TerminalWindowListOutput(
        windows=(
            TerminalWindowEntry(
                terminal_window_id=WINDOW_ID,
                window_id=42,
                process_id=84,
                shell="powershell",
                cwd="C:/project",
                title="Codex Pro Terminal",
            ),
        )
    )

    adapter: TypeAdapter[ProjectOperationOutput] = TypeAdapter(
        ProjectOperationOutput
    )
    parsed = adapter.validate_json(
        output.model_dump_json()
    )

    assert isinstance(parsed, TerminalWindowListOutput)
    assert parsed.windows[0].terminal_window_id == WINDOW_ID
    assert parsed.windows[0].running is True


@pytest.mark.parametrize(
    "payload",
    [
        {"kind": "terminal_window_open", "shell": "bash"},
        {"kind": "terminal_window_open", "cwd": ""},
        {"kind": "terminal_window_list", "unexpected": True},
        {"kind": "terminal_window_close", "terminal_window_id": "termwin_bad"},
    ],
)
def test_terminal_window_protocol_rejects_invalid_inputs(
    payload: dict[str, object],
) -> None:
    adapter: TypeAdapter[ProjectOperation] = TypeAdapter(ProjectOperation)
    with pytest.raises(ValidationError):
        _ = adapter.validate_python(payload)


def test_terminal_window_type_guard_excludes_background_exec() -> None:
    assert is_terminal_window_request(TerminalWindowListRequest())
    assert not is_terminal_window_request(TerminalExecRequest(command="echo ok"))
