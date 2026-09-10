from __future__ import annotations

import os
import queue
import subprocess
import threading
import time
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import IO, Final, Protocol

from codex_remote_mcp_subprocess_capture import (
    BoundedProcessCapture,
    TRUNCATION_MARKER,
)
from codex_remote_mcp_subprocess_platform import (
    close_process_pipes as _close_pipes,
    kill_direct_process as _kill_direct_process,
    start_owned_process as _start_process,
    terminate_owned_tree as _terminate_owned_tree,
)
from codex_windows_job import create_kill_on_close_job_for_suspended_process

_READ_BYTES: Final = 64 * 1024
_POLL_SECONDS: Final = 0.02
_CLEANUP_TIMEOUT_SECONDS: Final = 5.0


class CancellationSignal(Protocol):
    def is_set(self) -> bool: ...

    def wait(self, timeout: float | None = None) -> bool: ...


class RemoteProcessCancelled(RuntimeError):
    """Raised after caller cancellation has cleaned the owned process tree."""


@dataclass(frozen=True, slots=True)
class RemoteProcessResult:
    args: tuple[str, ...]
    returncode: int
    stdout: bytes
    stderr: bytes
    stdout_truncated: bool
    stderr_truncated: bool


@dataclass(frozen=True, slots=True)
class OwnedProcessOutcome:
    process_id: int
    exit_code: int | None
    stdout: bytes
    stderr: bytes
    stdout_bytes: int
    stderr_bytes: int
    duration_ms: int
    timed_out: bool
    cancelled: bool
    stdout_truncated: bool
    stderr_truncated: bool


def run_owned_bounded_process(
    command: Sequence[str],
    *,
    cwd: Path,
    env: Mapping[str, str],
    timeout_seconds: float,
    max_stream_bytes: int,
    cancel_event: CancellationSignal | None = None,
) -> RemoteProcessResult:
    """Run one owned process tree while retaining bounded stdout and stderr."""
    outcome = execute_owned_bounded_process(
        command,
        cwd=cwd,
        env=env,
        timeout_seconds=timeout_seconds,
        max_stream_bytes=max_stream_bytes,
        cancel_event=cancel_event,
    )
    if outcome.timed_out:
        error = subprocess.TimeoutExpired(tuple(command), timeout_seconds)
        error.output = outcome.stdout
        error.stderr = outcome.stderr
        raise error
    if outcome.cancelled:
        raise RemoteProcessCancelled("remote process was cancelled")
    assert outcome.exit_code is not None
    return RemoteProcessResult(
        args=tuple(command),
        returncode=outcome.exit_code,
        stdout=outcome.stdout,
        stderr=outcome.stderr,
        stdout_truncated=outcome.stdout_truncated,
        stderr_truncated=outcome.stderr_truncated,
    )


def execute_owned_bounded_process(
    command: Sequence[str],
    *,
    cwd: Path,
    env: Mapping[str, str],
    timeout_seconds: float,
    max_stream_bytes: int,
    cancel_event: CancellationSignal | None = None,
) -> OwnedProcessOutcome:
    """Execute and clean one process tree, returning timeout/cancel outcomes."""
    if max_stream_bytes < len(TRUNCATION_MARKER) + 2:
        raise ValueError("max_stream_bytes is too small for bounded diagnostics")
    if cancel_event is not None and cancel_event.is_set():
        raise RemoteProcessCancelled("remote process was cancelled before startup")

    args = tuple(command)
    started_at = time.monotonic()
    process = _start_process(args, cwd=cwd, env=env)
    job = None
    try:
        job = (
            create_kill_on_close_job_for_suspended_process(process.pid)
            if os.name == "nt"
            else None
        )
    except BaseException:
        _kill_direct_process(process)
        _close_pipes(process)
        raise

    stdout_capture = BoundedProcessCapture(max_stream_bytes)
    stderr_capture = BoundedProcessCapture(max_stream_bytes)
    reader_errors: queue.SimpleQueue[BaseException] = queue.SimpleQueue()
    readers: list[threading.Thread] = []
    try:
        readers.append(_start_reader(process.stdout, stdout_capture, reader_errors))
        readers.append(_start_reader(process.stderr, stderr_capture, reader_errors))
    except BaseException as exc:
        cleanup_error = _terminate_owned_tree(process, job)
        _close_pipes(process)
        for reader in readers:
            reader.join(timeout=_CLEANUP_TIMEOUT_SECONDS)
        if cleanup_error is not None:
            exc.add_note(f"secondary cleanup failure: {type(cleanup_error).__name__}")
        raise
    primary: BaseException | None = None
    timed_out = False
    cancelled = False
    returncode: int | None = None
    try:
        while True:
            if not reader_errors.empty():
                primary = reader_errors.get()
                break
            if cancel_event is not None and cancel_event.is_set():
                cancelled = True
                break
            elapsed = time.monotonic() - started_at
            if elapsed >= timeout_seconds:
                timed_out = True
                break
            returncode = process.poll()
            if returncode is not None:
                break
            if cancel_event is None:
                time.sleep(min(_POLL_SECONDS, max(0.0, timeout_seconds - elapsed)))
            else:
                _ = cancel_event.wait(
                    min(_POLL_SECONDS, max(0.0, timeout_seconds - elapsed))
                )

        cleanup_error = _terminate_owned_tree(process, job)
        reader_error = _finish_readers(process, tuple(readers), reader_errors)
        stdout = stdout_capture.value()
        stderr = stderr_capture.value()
        failure = primary or reader_error or cleanup_error
        if failure is not None:
            secondary = cleanup_error if failure is not cleanup_error else reader_error
            if secondary is not None:
                failure.add_note(
                    f"secondary cleanup failure: {type(secondary).__name__}"
                )
            raise failure
        if not (timed_out or cancelled):
            assert returncode is not None
        return OwnedProcessOutcome(
            process_id=process.pid,
            exit_code=None if timed_out or cancelled else returncode,
            stdout=stdout,
            stderr=stderr,
            stdout_bytes=stdout_capture.total_bytes,
            stderr_bytes=stderr_capture.total_bytes,
            duration_ms=int((time.monotonic() - started_at) * 1_000),
            timed_out=timed_out,
            cancelled=cancelled,
            stdout_truncated=stdout_capture.truncated,
            stderr_truncated=stderr_capture.truncated,
        )
    finally:
        _close_pipes(process)


def _start_reader(
    pipe: IO[bytes] | None,
    capture: BoundedProcessCapture,
    errors: queue.SimpleQueue[BaseException],
) -> threading.Thread:
    assert pipe is not None

    def drain() -> None:
        try:
            while chunk := pipe.read(_READ_BYTES):
                capture.append(chunk)
        except BaseException as exc:  # noqa: BLE001 - forwarded to the owner thread.
            errors.put(exc)

    reader = threading.Thread(target=drain, daemon=True)
    reader.start()
    return reader


def _finish_readers(
    process: subprocess.Popen[bytes],
    readers: Sequence[threading.Thread],
    errors: queue.SimpleQueue[BaseException],
) -> BaseException | None:
    for reader in readers:
        reader.join(timeout=_CLEANUP_TIMEOUT_SECONDS)
    if any(reader.is_alive() for reader in readers):
        _close_pipes(process)
        return TimeoutError("timed out draining owned process output")
    return errors.get() if not errors.empty() else None


__all__ = [
    "CancellationSignal",
    "OwnedProcessOutcome",
    "RemoteProcessCancelled",
    "RemoteProcessResult",
    "execute_owned_bounded_process",
    "run_owned_bounded_process",
]
