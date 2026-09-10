from __future__ import annotations

import ctypes
import os
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest

import codex_remote_mcp_subprocess as owned_subprocess
from codex_remote_mcp_subprocess import (
    RemoteProcessCancelled,
    run_owned_bounded_process,
)

_CREATE_NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)
_PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
_STILL_ACTIVE = 259


@pytest.mark.skipif(os.name != "nt", reason="Windows Job Object semantics")
def test_timeout_caps_output_and_removes_only_owned_process_tree(
    tmp_path: Path,
) -> None:
    pid_path = tmp_path / "tree.pids"
    activity_path = tmp_path / "activity.bin"
    sentinel = subprocess.Popen(
        [sys.executable, "-c", "import time; time.sleep(300)"],
        creationflags=_CREATE_NO_WINDOW,
    )
    try:
        with pytest.raises(subprocess.TimeoutExpired) as caught:
            _ = run_owned_bounded_process(
                _flooding_tree_command(pid_path, activity_path),
                cwd=tmp_path,
                env=os.environ.copy(),
                timeout_seconds=1,
                max_stream_bytes=4_096,
            )

        error = caught.value
        assert isinstance(error.output, bytes)
        assert isinstance(error.stderr, bytes)
        assert len(error.output) <= 4_096
        assert len(error.stderr) <= 4_096
        assert b"...[output truncated]..." in error.output
        assert b"...[output truncated]..." in error.stderr
        pids = _read_pids(pid_path)
        assert len(pids) == 3
        assert all(_wait_pid_gone(pid) for pid in pids)
        assert sentinel.poll() is None
        size_after_stop = activity_path.stat().st_size
        time.sleep(0.2)
        assert activity_path.stat().st_size == size_after_stop
    finally:
        sentinel.kill()
        _ = sentinel.wait(timeout=3)


@pytest.mark.skipif(os.name != "nt", reason="Windows Job Object semantics")
def test_cancellation_removes_owned_process_tree(tmp_path: Path) -> None:
    pid_path = tmp_path / "cancel.pids"
    cancel_event = threading.Event()
    errors: list[BaseException] = []

    def run_tree() -> None:
        try:
            _ = run_owned_bounded_process(
                _flooding_tree_command(pid_path, tmp_path / "cancel.bin"),
                cwd=tmp_path,
                env=os.environ.copy(),
                timeout_seconds=30,
                max_stream_bytes=4_096,
                cancel_event=cancel_event,
            )
        except BaseException as exc:  # noqa: BLE001 - the test records thread failure.
            errors.append(exc)

    worker = threading.Thread(target=run_tree)
    worker.start()
    try:
        _wait_for_tree(pid_path)
        cancel_event.set()
        worker.join(timeout=5)
        assert not worker.is_alive()
        assert len(errors) == 1
        assert isinstance(errors[0], RemoteProcessCancelled)
        assert all(_wait_pid_gone(pid) for pid in _read_pids(pid_path))
    finally:
        cancel_event.set()
        worker.join(timeout=5)


@pytest.mark.skipif(os.name != "nt", reason="Windows Job Object semantics")
def test_parent_exit_still_removes_descendant_holding_output_pipe(
    tmp_path: Path,
) -> None:
    child_pid_path = tmp_path / "child.pid"
    child_code = (
        "import os,pathlib,time; "
        f"pathlib.Path({str(child_pid_path)!r}).write_text(str(os.getpid())); "
        "time.sleep(300)"
    )
    parent_code = (
        "import pathlib,subprocess,sys,time; "
        f"subprocess.Popen([sys.executable,'-c',{child_code!r}]); "
        f"p=pathlib.Path({str(child_pid_path)!r}); "
        "deadline=time.monotonic()+2; "
        'exec("while not p.exists() and time.monotonic() < deadline:\\n time.sleep(0.01)"); '
        "print('done')"
    )

    result = run_owned_bounded_process(
        (sys.executable, "-c", parent_code),
        cwd=tmp_path,
        env=os.environ.copy(),
        timeout_seconds=5,
        max_stream_bytes=4_096,
    )

    assert result.returncode == 0
    child_pid = int(child_pid_path.read_text(encoding="ascii"))
    assert _wait_pid_gone(child_pid)


@pytest.mark.skipif(os.name != "nt", reason="Windows Job Object semantics")
def test_job_assignment_failure_never_resumes_suspended_parent(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    sentinel = tmp_path / "should-not-exist.txt"

    def fail_assignment(_pid: int):
        raise OSError("injected assignment failure")

    monkeypatch.setattr(
        owned_subprocess,
        "create_kill_on_close_job_for_suspended_process",
        fail_assignment,
    )

    with pytest.raises(OSError, match="injected assignment failure"):
        _ = run_owned_bounded_process(
            (
                sys.executable,
                "-c",
                f"from pathlib import Path; Path({str(sentinel)!r}).write_text('ran')",
            ),
            cwd=tmp_path,
            env=os.environ.copy(),
            timeout_seconds=5,
            max_stream_bytes=4_096,
        )

    assert not sentinel.exists()


@pytest.mark.skipif(os.name != "nt", reason="Windows Job Object semantics")
def test_reader_start_failure_cleans_the_resumed_process(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    started_pid: list[int] = []
    original_start = owned_subprocess._start_process

    def tracked_start(*args, **kwargs):
        process = original_start(*args, **kwargs)
        started_pid.append(process.pid)
        return process

    def fail_reader_start(*_args, **_kwargs):
        raise RuntimeError("injected reader start failure")

    monkeypatch.setattr(owned_subprocess, "_start_process", tracked_start)
    monkeypatch.setattr(owned_subprocess, "_start_reader", fail_reader_start)

    with pytest.raises(RuntimeError, match="injected reader start failure"):
        _ = run_owned_bounded_process(
            (sys.executable, "-c", "import time; time.sleep(30)"),
            cwd=tmp_path,
            env=os.environ.copy(),
            timeout_seconds=30,
            max_stream_bytes=4_096,
        )

    assert len(started_pid) == 1
    assert _wait_pid_gone(started_pid[0])


def _flooding_tree_command(pid_path: Path, activity_path: Path) -> tuple[str, ...]:
    grandchild_code = "\n".join(
        (
            "import os, pathlib, sys, time",
            f"pid_path = pathlib.Path({str(pid_path)!r})",
            "with pid_path.open('a', encoding='ascii') as stream:",
            "    stream.write(f'{os.getpid()}\\n')",
            f"activity = open({str(activity_path)!r}, 'ab', buffering=0)",
            "while True:",
            "    sys.stdout.buffer.write(b'g' * 65536); sys.stdout.buffer.flush()",
            "    sys.stderr.buffer.write(b'e' * 65536); sys.stderr.buffer.flush()",
            "    activity.write(b'x')",
            "    time.sleep(0.01)",
        )
    )
    child_code = "\n".join(
        (
            "import os, pathlib, subprocess, sys, time",
            f"subprocess.Popen([sys.executable, '-u', '-c', {grandchild_code!r}])",
            f"pid_path = pathlib.Path({str(pid_path)!r})",
            "with pid_path.open('a', encoding='ascii') as stream:",
            "    stream.write(f'{os.getpid()}\\n')",
            "while True:",
            "    sys.stdout.buffer.write(b'c' * 65536); sys.stdout.buffer.flush()",
            "    time.sleep(0.01)",
        )
    )
    parent_code = "\n".join(
        (
            "import os, pathlib, subprocess, sys, time",
            f"subprocess.Popen([sys.executable, '-u', '-c', {child_code!r}])",
            f"pid_path = pathlib.Path({str(pid_path)!r})",
            "with pid_path.open('a', encoding='ascii') as stream:",
            "    stream.write(f'{os.getpid()}\\n')",
            "while True:",
            "    sys.stdout.buffer.write(b'p' * 65536); sys.stdout.buffer.flush()",
            "    time.sleep(0.01)",
        )
    )
    return (sys.executable, "-u", "-c", parent_code)


def _wait_for_tree(path: Path) -> None:
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline and len(_read_pids(path)) < 3:
        time.sleep(0.02)
    assert len(_read_pids(path)) == 3


def _read_pids(path: Path) -> tuple[int, ...]:
    if not path.exists():
        return ()
    return tuple(
        int(line)
        for line in path.read_text(encoding="ascii").splitlines()
        if line.isdigit()
    )


def _pid_exists(pid: int) -> bool:
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.OpenProcess.argtypes = [ctypes.c_uint32, ctypes.c_int, ctypes.c_uint32]
    kernel32.OpenProcess.restype = ctypes.c_void_p
    kernel32.GetExitCodeProcess.argtypes = [
        ctypes.c_void_p,
        ctypes.POINTER(ctypes.c_uint32),
    ]
    kernel32.GetExitCodeProcess.restype = ctypes.c_int
    kernel32.CloseHandle.argtypes = [ctypes.c_void_p]
    kernel32.CloseHandle.restype = ctypes.c_int
    handle = kernel32.OpenProcess(_PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    if not handle:
        return ctypes.get_last_error() == 5
    exit_code = ctypes.c_uint32()
    try:
        if not kernel32.GetExitCodeProcess(handle, ctypes.byref(exit_code)):
            return True
        return int(exit_code.value) == _STILL_ACTIVE
    finally:
        _ = kernel32.CloseHandle(handle)


def _wait_pid_gone(pid: int) -> bool:
    deadline = time.monotonic() + 3
    while time.monotonic() < deadline:
        if not _pid_exists(pid):
            return True
        time.sleep(0.05)
    return not _pid_exists(pid)
