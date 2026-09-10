from __future__ import annotations

from datetime import datetime
from typing import Annotated, ClassVar, Literal, Self, TypeGuard

from pydantic import ConfigDict, Field, model_validator

from simdorei_mcp_common.operation_base import OperationOutput, OperationRequest
from simdorei_mcp_common.terminal_window_protocol import (
    TerminalWindowEntry,
    TerminalWindowId,
)

TerminalWindowObservationId = Annotated[
    str,
    Field(pattern=r"^twobs_[a-f0-9]{16}$"),
]
TerminalWindowReceiptId = Annotated[
    str,
    Field(pattern=r"^twrcpt_[a-f0-9]{16}$"),
]
TerminalWindowKey = Annotated[str, Field(min_length=1, max_length=20)]
TerminalWindowAction = Literal["activate", "type", "keys", "interrupt"]


class TerminalWindowCaptureRequest(OperationRequest):
    kind: Literal["terminal_window_capture"] = "terminal_window_capture"
    terminal_window_id: TerminalWindowId


class TerminalWindowActivateRequest(OperationRequest):
    kind: Literal["terminal_window_activate"] = "terminal_window_activate"
    terminal_window_id: TerminalWindowId


class TerminalWindowTypeRequest(OperationRequest):
    kind: Literal["terminal_window_type"] = "terminal_window_type"
    terminal_window_id: TerminalWindowId
    observation_id: TerminalWindowObservationId
    text: str = Field(min_length=1, max_length=4_096)


class TerminalWindowKeysRequest(OperationRequest):
    kind: Literal["terminal_window_keys"] = "terminal_window_keys"
    terminal_window_id: TerminalWindowId
    observation_id: TerminalWindowObservationId
    keys: tuple[TerminalWindowKey, ...] = Field(min_length=1, max_length=4)


class TerminalWindowInterruptRequest(OperationRequest):
    kind: Literal["terminal_window_interrupt"] = "terminal_window_interrupt"
    terminal_window_id: TerminalWindowId
    observation_id: TerminalWindowObservationId


class TerminalWindowRect(OperationOutput):
    left: int
    top: int
    width: int = Field(gt=0)
    height: int = Field(gt=0)


class TerminalWindowCaptureOutput(OperationOutput):
    kind: Literal["terminal_window_capture"] = "terminal_window_capture"
    window: TerminalWindowEntry
    observation_id: TerminalWindowObservationId
    identity_digest: str = Field(pattern=r"^[a-f0-9]{64}$")
    rect: TerminalWindowRect
    media_type: Literal["image/png"] = "image/png"
    data_base64: str = Field(min_length=12, max_length=12_000_000)
    captured_at: datetime


class TerminalWindowActionReceipt(OperationOutput):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")

    receipt_id: TerminalWindowReceiptId
    terminal_window_id: TerminalWindowId
    observation_id: TerminalWindowObservationId | None = None
    identity_digest: str = Field(pattern=r"^[a-f0-9]{64}$")
    action: TerminalWindowAction
    unicode_chars: int = Field(default=0, ge=0, le=4_096)
    keys: tuple[TerminalWindowKey, ...] = Field(default=(), max_length=4)
    activated: bool
    completed_at: datetime

    @model_validator(mode="after")
    def require_action_shape(self) -> Self:
        if self.action == "activate":
            valid = self.observation_id is None and not self.unicode_chars and not self.keys
        elif self.action == "type":
            valid = (
                self.observation_id is not None
                and self.unicode_chars > 0
                and not self.keys
            )
        elif self.action == "keys":
            valid = (
                self.observation_id is not None
                and not self.unicode_chars
                and bool(self.keys)
            )
        else:
            valid = (
                self.observation_id is not None
                and not self.unicode_chars
                and self.keys == ("CTRL", "C")
            )
        if not valid:
            raise ValueError("terminal window action receipt fields are inconsistent")
        return self


class TerminalWindowActionOutput(OperationOutput):
    kind: Literal["terminal_window_action"] = "terminal_window_action"
    window: TerminalWindowEntry
    receipt: TerminalWindowActionReceipt

    @model_validator(mode="after")
    def require_matching_window_identity(self) -> Self:
        if self.receipt.terminal_window_id != self.window.terminal_window_id:
            raise ValueError("terminal window receipt identity does not match output")
        return self


TerminalWindowInteractionRequest = (
    TerminalWindowCaptureRequest
    | TerminalWindowActivateRequest
    | TerminalWindowTypeRequest
    | TerminalWindowKeysRequest
    | TerminalWindowInterruptRequest
)
TerminalWindowInteractionOutput = (
    TerminalWindowCaptureOutput | TerminalWindowActionOutput
)


def is_terminal_window_interaction_request(
    value: object,
) -> TypeGuard[TerminalWindowInteractionRequest]:
    return isinstance(
        value,
        (
            TerminalWindowCaptureRequest,
            TerminalWindowActivateRequest,
            TerminalWindowTypeRequest,
            TerminalWindowKeysRequest,
            TerminalWindowInterruptRequest,
        ),
    )


__all__ = [
    "TerminalWindowActionOutput",
    "TerminalWindowActionReceipt",
    "TerminalWindowActivateRequest",
    "TerminalWindowCaptureOutput",
    "TerminalWindowCaptureRequest",
    "TerminalWindowInteractionOutput",
    "TerminalWindowInteractionRequest",
    "TerminalWindowInterruptRequest",
    "TerminalWindowKeysRequest",
    "TerminalWindowObservationId",
    "TerminalWindowReceiptId",
    "TerminalWindowRect",
    "TerminalWindowTypeRequest",
    "is_terminal_window_interaction_request",
]
