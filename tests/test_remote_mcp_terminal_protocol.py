from __future__ import annotations

import json

import pytest
from pydantic import TypeAdapter, ValidationError

from simdorei_mcp_common.operation_requests import ProjectOperation
from simdorei_mcp_common.terminal_protocol import (
    TerminalExecutionReceipt,
    TerminalExecOutput,
    TerminalExecRequest,
)


TERMINAL_ID = "term_0123456789abcdef"
RECEIPT_ID = "tr_0123456789abcdef"
COMMAND_DIGEST = "a" * 64


def _receipt(**changes: object) -> TerminalExecutionReceipt:
    values: dict[str, object] = {
        "receipt_id": RECEIPT_ID,
        "terminal_id": TERMINAL_ID,
        "command_digest": COMMAND_DIGEST,
        "shell": "powershell",
        "cwd_scope": "project_relative",
        "exit_code": 0,
        "stdout_bytes": 2,
        "stderr_bytes": 0,
        "duration_ms": 10,
        "timed_out": False,
        "cancelled": False,
        "truncated": False,
    }
    values.update(changes)
    return TerminalExecutionReceipt.model_validate(values)


def test_terminal_request_accepts_arbitrary_command_contract() -> None:
    request = TerminalExecRequest(
        shell="powershell",
        command="py -3 -c \"print('ok')\"",
        cwd="tests",
        environment={"QA_MODE": "1"},
        timeout_seconds=600,
    )

    assert request.kind == "terminal_exec"
    assert request.terminal_id is None
    assert request.cancel_previous is False
    assert request.environment == {"QA_MODE": "1"}


def test_terminal_request_rejects_invalid_bounds_and_ids() -> None:
    cases = (
        {"command": ""},
        {"command": "echo ok", "terminal_id": "not-a-terminal"},
        {"command": "echo ok", "timeout_seconds": 0},
        {"command": "echo ok", "environment": {str(index): "x" for index in range(101)}},
        {"command": "echo ok", "environment": {"": "x"}},
        {"command": "echo ok", "environment": {"BAD=NAME": "x"}},
        {"command": "echo ok", "environment": {"BAD\x00NAME": "x"}},
        {"command": "echo ok", "environment": {"GOOD_NAME": "bad\x00value"}},
        {"command": "echo ok", "environment": {"GOOD_NAME": "x" * 32_768}},
    )
    for values in cases:
        with pytest.raises(ValidationError):
            _ = TerminalExecRequest.model_validate(values)


def test_terminal_output_requires_a_consistent_secret_free_receipt() -> None:
    output = TerminalExecOutput(
        terminal_id=TERMINAL_ID,
        process_id=123,
        exit_code=0,
        stdout="ok",
        stderr="",
        cwd="tests",
        duration_ms=10,
        timed_out=False,
        cancelled=False,
        truncated=False,
        receipt=_receipt(),
    )

    receipt_payload = output.receipt.model_dump()
    assert set(receipt_payload) == {
        "receipt_id",
        "terminal_id",
        "command_digest",
        "shell",
        "cwd_scope",
        "exit_code",
        "stdout_bytes",
        "stderr_bytes",
        "duration_ms",
        "timed_out",
        "cancelled",
        "truncated",
    }
    serialized = output.receipt.model_dump_json()
    assert output.receipt.command_digest == COMMAND_DIGEST
    for forbidden in ("py -3", "C:/private", "project_scope", "token", "cookie"):
        assert forbidden not in serialized

    with pytest.raises(ValidationError):
        _ = TerminalExecOutput.model_validate(
            {
                **output.model_dump(),
                "exit_code": 7,
            }
        )


def test_terminal_receipt_requires_exit_or_single_stop_reason() -> None:
    with pytest.raises(ValidationError):
        _ = _receipt(exit_code=None)
    with pytest.raises(ValidationError):
        _ = _receipt(exit_code=None, timed_out=True, cancelled=True)

    timed_out = _receipt(exit_code=None, timed_out=True)
    assert timed_out.timed_out is True


def test_terminal_models_are_advertised_after_execution_is_wired() -> None:
    adapter: TypeAdapter[ProjectOperation] = TypeAdapter(ProjectOperation)

    operation = adapter.validate_json(
        json.dumps({"kind": "terminal_exec", "command": "echo wired"})
    )

    assert isinstance(operation, TerminalExecRequest)
