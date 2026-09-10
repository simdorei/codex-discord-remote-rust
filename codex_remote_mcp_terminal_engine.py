from __future__ import annotations

import hashlib
import secrets
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Final, final

from codex_remote_mcp_redaction import redact
from codex_remote_mcp_files import ProjectFileError
from codex_remote_mcp_subprocess import (
    CancellationSignal,
    RemoteProcessCancelled,
    execute_owned_bounded_process,
)
from codex_remote_mcp_terminal_runtime import (
    CombinedCancellation,
    inherited_terminal_environment,
    resolve_terminal_cwd,
    terminal_cwd_scope,
    terminal_shell_argv,
)
from simdorei_mcp_common.terminal_protocol import (
    TerminalExecutionReceipt,
    TerminalExecOutput,
    TerminalExecRequest,
)

_MAX_STREAM_BYTES: Final = 1_048_576
_CLOSE_TIMEOUT_SECONDS: Final = 10.0


class TerminalExecutionError(ProjectFileError):
    """A public-safe failure in a session-owned terminal execution."""

    def __init__(self, reason: str) -> None:
        super().__init__("terminal", reason)


@dataclass(slots=True)
class _ExecutionTicket:
    cancel: threading.Event = field(default_factory=threading.Event)
    done: threading.Event = field(default_factory=threading.Event)


@dataclass(slots=True)
class _TerminalState:
    terminal_id: str
    cwd: Path
    environment: dict[str, str]
    active: _ExecutionTicket | None = None
    closed: bool = False


@final
class TerminalExecutionEngine:
    """Own arbitrary shell executions for exactly one selected MCP session."""

    def __init__(self, root: Path, *, session_id: str) -> None:
        if not session_id:
            raise ValueError("session_id must not be empty")
        self._root: Path = root.resolve(strict=True)
        if not self._root.is_dir():
            raise ValueError("terminal root must be a directory")
        self._session_id: str = session_id
        self._lock: threading.RLock = threading.RLock()
        self._terminals: dict[str, _TerminalState] = {}
        self._closed: bool = False

    @property
    def session_id(self) -> str:
        return self._session_id

    @property
    def root(self) -> Path:
        return self._root

    def execute(
        self,
        request: TerminalExecRequest,
        *,
        cancel_event: CancellationSignal | None = None,
        timeout_seconds: float | None = None,
    ) -> TerminalExecOutput:
        if cancel_event is not None and cancel_event.is_set():
            raise TerminalExecutionError("terminal execution was cancelled")
        state = self._get_or_create_terminal(request.terminal_id)
        ticket = self._reserve(
            state,
            cancel_previous=request.cancel_previous,
            cancel_event=cancel_event,
        )
        try:
            cwd = resolve_terminal_cwd(state.cwd, request.cwd)
            environment = {**state.environment, **request.environment}
            shell, argv = terminal_shell_argv(request.shell, request.command)
            outcome = execute_owned_bounded_process(
                argv,
                cwd=cwd,
                env=environment,
                timeout_seconds=min(
                    request.timeout_seconds,
                    timeout_seconds
                    if timeout_seconds is not None
                    else request.timeout_seconds,
                ),
                max_stream_bytes=_MAX_STREAM_BYTES,
                cancel_event=CombinedCancellation(ticket.cancel, cancel_event),
            )
            state.cwd = cwd
            state.environment = environment
            truncated = outcome.stdout_truncated or outcome.stderr_truncated
            receipt = TerminalExecutionReceipt(
                receipt_id=f"tr_{secrets.token_hex(8)}",
                terminal_id=state.terminal_id,
                command_digest=hashlib.sha256(request.command.encode()).hexdigest(),
                shell=shell,
                cwd_scope=terminal_cwd_scope(self._root, cwd),
                exit_code=outcome.exit_code,
                stdout_bytes=outcome.stdout_bytes,
                stderr_bytes=outcome.stderr_bytes,
                duration_ms=outcome.duration_ms,
                timed_out=outcome.timed_out,
                cancelled=outcome.cancelled,
                truncated=truncated,
            )
            return TerminalExecOutput(
                terminal_id=state.terminal_id,
                process_id=outcome.process_id,
                exit_code=outcome.exit_code,
                stdout=redact(outcome.stdout.decode("utf-8", errors="replace")),
                stderr=redact(outcome.stderr.decode("utf-8", errors="replace")),
                cwd=str(cwd),
                duration_ms=outcome.duration_ms,
                timed_out=outcome.timed_out,
                cancelled=outcome.cancelled,
                truncated=truncated,
                receipt=receipt,
            )
        except RemoteProcessCancelled as exc:
            raise TerminalExecutionError("terminal execution was cancelled") from exc
        except (FileNotFoundError, NotADirectoryError) as exc:
            raise TerminalExecutionError("terminal executable or directory was not found") from exc
        except OSError as exc:
            raise TerminalExecutionError(
                f"terminal process could not start ({type(exc).__name__})"
            ) from exc
        finally:
            self._release(state, ticket)

    def cancel(self, terminal_id: str) -> bool:
        with self._lock:
            state = self._terminals.get(terminal_id)
            if state is None or state.active is None:
                return False
            state.active.cancel.set()
            return True

    def list_terminal_ids(self) -> tuple[str, ...]:
        with self._lock:
            return tuple(sorted(self._terminals))

    def close(self) -> None:
        with self._lock:
            if self._closed:
                return
            self._closed = True
            states = tuple(self._terminals.values())
            active = tuple(state.active for state in states if state.active is not None)
            for state in states:
                state.closed = True
            for ticket in active:
                ticket.cancel.set()
        deadline = time.monotonic() + _CLOSE_TIMEOUT_SECONDS
        for ticket in active:
            if not ticket.done.wait(max(0.0, deadline - time.monotonic())):
                raise TerminalExecutionError("timed out closing terminal process trees")
        with self._lock:
            self._terminals.clear()

    def _get_or_create_terminal(self, terminal_id: str | None) -> _TerminalState:
        with self._lock:
            if self._closed:
                raise TerminalExecutionError("terminal session is closed")
            if terminal_id is not None:
                state = self._terminals.get(terminal_id)
                if state is None or state.closed:
                    raise TerminalExecutionError("terminal does not belong to this session")
                return state
            while True:
                generated = f"term_{secrets.token_hex(8)}"
                if generated not in self._terminals:
                    break
            state = _TerminalState(
                terminal_id=generated,
                cwd=self._root,
                environment=inherited_terminal_environment(),
            )
            self._terminals[generated] = state
            return state

    def _reserve(
        self,
        state: _TerminalState,
        *,
        cancel_previous: bool,
        cancel_event: CancellationSignal | None,
    ) -> _ExecutionTicket:
        while True:
            with self._lock:
                if self._closed or state.closed:
                    raise TerminalExecutionError("terminal session is closed")
                if state.active is None:
                    ticket = _ExecutionTicket()
                    state.active = ticket
                    return ticket
                previous = state.active
                if not cancel_previous:
                    raise TerminalExecutionError("terminal already has an active command")
                previous.cancel.set()
            while not previous.done.wait(0.02):
                if cancel_event is not None and cancel_event.is_set():
                    raise TerminalExecutionError("terminal execution was cancelled")

    def _release(self, state: _TerminalState, ticket: _ExecutionTicket) -> None:
        with self._lock:
            if state.active is ticket:
                state.active = None
        ticket.done.set()

__all__ = ["TerminalExecutionEngine", "TerminalExecutionError"]
