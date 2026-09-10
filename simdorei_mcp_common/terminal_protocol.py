from __future__ import annotations

from typing import Annotated, ClassVar, Literal, Self

from pydantic import BaseModel, ConfigDict, Field, model_validator

from simdorei_mcp_common.operation_base import OperationOutput, OperationRequest


TerminalId = Annotated[
    str,
    Field(pattern=r"^term_[a-f0-9]{16}$"),
]
EnvironmentName = Annotated[
    str,
    Field(min_length=1, max_length=1_024, pattern=r"^[^=\x00]+$"),
]
EnvironmentValue = Annotated[
    str,
    Field(max_length=32_767, pattern=r"^[^\x00]*$"),
]
TerminalShell = Literal["auto", "powershell", "cmd", "sh", "bash"]
TerminalCwdScope = Literal[
    "project_root",
    "project_relative",
    "external_absolute",
]


class TerminalExecRequest(OperationRequest):
    kind: Literal["terminal_exec"] = "terminal_exec"
    terminal_id: TerminalId | None = None
    shell: TerminalShell = "auto"
    command: str = Field(min_length=1, max_length=32_768)
    cwd: str | None = Field(default=None, min_length=1, max_length=1_000)
    environment: dict[EnvironmentName, EnvironmentValue] = Field(
        default_factory=dict,
        max_length=100,
    )
    timeout_seconds: int = Field(default=300, ge=1, le=3_600)
    cancel_previous: bool = False


class TerminalExecutionReceipt(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")

    receipt_id: str = Field(pattern=r"^tr_[a-f0-9]{16}$")
    terminal_id: TerminalId
    command_digest: str = Field(pattern=r"^[a-f0-9]{64}$")
    shell: TerminalShell
    cwd_scope: TerminalCwdScope
    exit_code: int | None
    stdout_bytes: int = Field(ge=0)
    stderr_bytes: int = Field(ge=0)
    duration_ms: int = Field(ge=0)
    timed_out: bool
    cancelled: bool
    truncated: bool

    @model_validator(mode="after")
    def require_terminal_outcome(self) -> Self:
        if self.timed_out and self.cancelled:
            raise ValueError("terminal execution cannot be timed out and cancelled")
        if self.exit_code is None and not (self.timed_out or self.cancelled):
            raise ValueError("terminal execution without an exit code needs a stop reason")
        return self


class TerminalExecOutput(OperationOutput):
    kind: Literal["terminal_exec"] = "terminal_exec"
    terminal_id: TerminalId
    process_id: int = Field(gt=0)
    exit_code: int | None
    stdout: str = Field(max_length=1_048_576)
    stderr: str = Field(max_length=1_048_576)
    cwd: str = Field(min_length=1, max_length=1_000)
    duration_ms: int = Field(ge=0)
    timed_out: bool
    cancelled: bool
    truncated: bool
    receipt: TerminalExecutionReceipt

    @model_validator(mode="after")
    def match_receipt(self) -> Self:
        if self.receipt.terminal_id != self.terminal_id:
            raise ValueError("terminal output and receipt IDs must match")
        if self.receipt.exit_code != self.exit_code:
            raise ValueError("terminal output and receipt exit codes must match")
        if self.receipt.duration_ms != self.duration_ms:
            raise ValueError("terminal output and receipt durations must match")
        if self.receipt.timed_out != self.timed_out:
            raise ValueError("terminal output and receipt timeout states must match")
        if self.receipt.cancelled != self.cancelled:
            raise ValueError("terminal output and receipt cancellation states must match")
        if self.receipt.truncated != self.truncated:
            raise ValueError("terminal output and receipt truncation states must match")
        return self
