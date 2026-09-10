from __future__ import annotations

from datetime import UTC, datetime

import pytest
from pydantic import TypeAdapter, ValidationError

from simdorei_mcp_common.operation_outputs import ProjectOperationOutput
from simdorei_mcp_common.operation_requests import ProjectOperation
from simdorei_mcp_common.terminal_window_interaction_protocol import (
    TerminalWindowActionOutput,
    TerminalWindowActionReceipt,
    TerminalWindowCaptureOutput,
    TerminalWindowKeysRequest,
    TerminalWindowRect,
    TerminalWindowTypeRequest,
    is_terminal_window_interaction_request,
)
from simdorei_mcp_common.terminal_window_protocol import TerminalWindowEntry

WINDOW_ID = "termwin_0123456789abcdef"
OBSERVATION_ID = "twobs_0123456789abcdef"


def test_interaction_requests_parse_through_project_union() -> None:
    adapter: TypeAdapter[ProjectOperation] = TypeAdapter(ProjectOperation)
    typed = adapter.validate_python(
        {
            "kind": "terminal_window_type",
            "terminal_window_id": WINDOW_ID,
            "observation_id": OBSERVATION_ID,
            "text": "echo hello",
        }
    )
    keys = adapter.validate_python(
        {
            "kind": "terminal_window_keys",
            "terminal_window_id": WINDOW_ID,
            "observation_id": OBSERVATION_ID,
            "keys": ["CTRL", "L"],
        }
    )

    assert isinstance(typed, TerminalWindowTypeRequest)
    assert isinstance(keys, TerminalWindowKeysRequest)
    assert is_terminal_window_interaction_request(typed)


def test_interaction_outputs_parse_through_project_union() -> None:
    captured = TerminalWindowCaptureOutput(
        window=_entry(),
        observation_id=OBSERVATION_ID,
        identity_digest="a" * 64,
        rect=TerminalWindowRect(left=1, top=2, width=640, height=480),
        data_base64="aGVsbG8gd29ybGQ=",
        captured_at=datetime.now(UTC),
    )
    adapter: TypeAdapter[ProjectOperationOutput] = TypeAdapter(ProjectOperationOutput)
    parsed = adapter.validate_json(captured.model_dump_json())

    assert isinstance(parsed, TerminalWindowCaptureOutput)
    assert parsed.observation_id == OBSERVATION_ID


def test_receipt_rejects_raw_text_and_inconsistent_action_shape() -> None:
    valid: dict[str, object] = {
        "receipt_id": "twrcpt_0123456789abcdef",
        "terminal_window_id": WINDOW_ID,
        "observation_id": OBSERVATION_ID,
        "identity_digest": "b" * 64,
        "action": "type",
        "unicode_chars": 5,
        "keys": (),
        "activated": False,
        "completed_at": datetime.now(UTC),
    }
    receipt = TerminalWindowActionReceipt.model_validate(valid)
    output = TerminalWindowActionOutput(window=_entry(), receipt=receipt)

    assert "text" not in output.model_dump_json()
    with pytest.raises(ValidationError):
        _ = TerminalWindowActionReceipt.model_validate({**valid, "text": "secret"})
    with pytest.raises(ValidationError, match="inconsistent"):
        _ = TerminalWindowActionReceipt.model_validate(
            {**valid, "unicode_chars": 0}
        )


def test_action_output_rejects_a_receipt_for_another_window() -> None:
    other_window_id = "termwin_fedcba9876543210"
    receipt = TerminalWindowActionReceipt(
        receipt_id="twrcpt_0123456789abcdef",
        terminal_window_id=other_window_id,
        observation_id=None,
        identity_digest="c" * 64,
        action="activate",
        activated=True,
        completed_at=datetime.now(UTC),
    )

    with pytest.raises(ValidationError, match="identity does not match"):
        _ = TerminalWindowActionOutput(window=_entry(), receipt=receipt)


def _entry() -> TerminalWindowEntry:
    return TerminalWindowEntry(
        terminal_window_id=WINDOW_ID,
        window_id=42,
        process_id=84,
        shell="cmd",
        cwd="C:/project",
        title="Codex Pro Terminal",
    )
