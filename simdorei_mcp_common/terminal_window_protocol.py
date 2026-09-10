from __future__ import annotations

from typing import Annotated, ClassVar, Literal, TypeGuard

from pydantic import BaseModel, ConfigDict, Field

from simdorei_mcp_common.operation_base import OperationOutput, OperationRequest
from simdorei_mcp_common.terminal_protocol import (
    TerminalExecOutput,
    TerminalExecRequest,
)


TerminalWindowId = Annotated[
    str,
    Field(pattern=r"^termwin_[a-f0-9]{16}$"),
]
TerminalWindowShell = Literal["powershell", "cmd"]


class TerminalWindowOpenRequest(OperationRequest):
    kind: Literal["terminal_window_open"] = "terminal_window_open"
    shell: TerminalWindowShell = "powershell"
    cwd: str | None = Field(default=None, min_length=1, max_length=1_000)


class TerminalWindowListRequest(OperationRequest):
    kind: Literal["terminal_window_list"] = "terminal_window_list"


class TerminalWindowCloseRequest(OperationRequest):
    kind: Literal["terminal_window_close"] = "terminal_window_close"
    terminal_window_id: TerminalWindowId


class TerminalWindowEntry(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")

    terminal_window_id: TerminalWindowId
    window_id: int = Field(gt=0)
    process_id: int = Field(gt=0)
    shell: TerminalWindowShell
    cwd: str = Field(min_length=1, max_length=1_000)
    title: str = Field(min_length=1, max_length=500)
    running: Literal[True] = True


class TerminalWindowOpenOutput(OperationOutput):
    kind: Literal["terminal_window_open"] = "terminal_window_open"
    window: TerminalWindowEntry


class TerminalWindowListOutput(OperationOutput):
    kind: Literal["terminal_window_list"] = "terminal_window_list"
    windows: tuple[TerminalWindowEntry, ...]


class TerminalWindowCloseOutput(OperationOutput):
    kind: Literal["terminal_window_close"] = "terminal_window_close"
    terminal_window_id: TerminalWindowId
    closed: Literal[True] = True


TerminalWindowRequest = (
    TerminalWindowOpenRequest | TerminalWindowListRequest | TerminalWindowCloseRequest
)
TerminalWindowOutput = (
    TerminalWindowOpenOutput | TerminalWindowListOutput | TerminalWindowCloseOutput
)
TerminalOperationRequest = TerminalExecRequest | TerminalWindowRequest
TerminalOperationOutput = TerminalExecOutput | TerminalWindowOutput


def is_terminal_window_request(value: object) -> TypeGuard[TerminalWindowRequest]:
    return isinstance(
        value,
        (
            TerminalWindowOpenRequest,
            TerminalWindowListRequest,
            TerminalWindowCloseRequest,
        ),
    )


__all__ = [
    "TerminalWindowCloseOutput",
    "TerminalWindowCloseRequest",
    "TerminalWindowEntry",
    "TerminalWindowId",
    "TerminalWindowListOutput",
    "TerminalWindowListRequest",
    "TerminalWindowOpenOutput",
    "TerminalWindowOpenRequest",
    "TerminalWindowOutput",
    "TerminalWindowRequest",
    "TerminalWindowShell",
    "TerminalOperationOutput",
    "TerminalOperationRequest",
    "is_terminal_window_request",
]
