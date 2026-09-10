from __future__ import annotations

import os
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest

from codex_remote_mcp_subprocess import (
    OwnedProcessOutcome,
    execute_owned_bounded_process,
)
from codex_remote_mcp_terminal_engine import TerminalExecutionEngine
from simdorei_mcp_common.terminal_protocol import TerminalExecRequest

pytestmark = pytest.mark.skipif(os.name == "nt", reason="POSIX process-group semantics")


def test_posix_timeout_removes_owned_descendants_but_not_unrelated_process(
    tmp_path: Path,
) -> None:
    pid_path = tmp_path / "timeout.pids"
    activity_path = tmp_path / "timeout.activity"
    sentinel = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(300)"])
    try:
        result = execute_owned_bounded_process(
            _tree_command(pid_path, activity_path),
            cwd=tmp_path,
            env=os.environ.copy(),
            timeout_seconds=1,
            max_stream_bytes=4_096,
        )

        assert result.timed_out is True
        assert result.cancelled is False
        pids = _read_pids(pid_path)
        assert len(pids) == 3
        assert all(_wait_pid_stopped(pid) for pid in pids)
        assert sentinel.poll() is None
        size_after_stop = activity_path.stat().st_size
        time.sleep(0.2)
        assert activity_path.stat().st_size == size_after_stop
    finally:
        sentinel.kill()
        _ = sentinel.wait(timeout=3)


def test_posix_cancellation_removes_the_whole_owned_process_group(
    tmp_path: Path,
) -> None:
    pid_path = tmp_path / "cancel.pids"
    activity_path = tmp_path / "cancel.activity"
    cancelled = threading.Event()
    outcomes: list[OwnedProcessOutcome] = []

    worker = threading.Thread(
        target=lambda: outcomes.append(
            execute_owned_bounded_process(
                _tree_command(pid_path, activity_path),
                cwd=tmp_path,
                env=os.environ.copy(),
                timeout_seconds=30,
                max_stream_bytes=4_096,
                cancel_event=cancelled,
            )
        )
    )
    worker.start()
    _wait_for_tree(pid_path)
    cancelled.set()
    worker.join(timeout=10)

    assert not worker.is_alive()
    assert len(outcomes) == 1
    assert outcomes[0].cancelled is True
    assert outcomes[0].timed_out is False
    assert all(_wait_pid_stopped(pid) for pid in _read_pids(pid_path))


def test_posix_shell_execution_uses_explicit_bash(tmp_path: Path) -> None:
    engine = TerminalExecutionEngine(tmp_path, session_id="posix-shell")

    result = engine.execute(
        TerminalExecRequest(
            shell="bash",
            command='test -n "$BASH_VERSION" && printf "bash-shell-ok"',
        )
    )

    assert result.exit_code == 0
    assert result.stdout == "bash-shell-ok"
    assert result.receipt.shell == "bash"


def _tree_command(pid_path: Path, activity_path: Path) -> tuple[str, ...]:
    grandchild = "\n".join(
        (
            "import os, pathlib, time",
            f"p = pathlib.Path({str(pid_path)!r})",
            "with p.open('a', encoding='ascii') as stream:",
            "    stream.write(f'{os.getpid()}\\n')",
            f"activity = open({str(activity_path)!r}, 'ab', buffering=0)",
            "while True:",
            "    activity.write(b'g')",
            "    time.sleep(0.01)",
        )
    )
    child = "\n".join(
        (
            "import os, pathlib, subprocess, sys, time",
            f"subprocess.Popen([sys.executable, '-c', {grandchild!r}])",
            f"p = pathlib.Path({str(pid_path)!r})",
            "with p.open('a', encoding='ascii') as stream:",
            "    stream.write(f'{os.getpid()}\\n')",
            "while True: time.sleep(1)",
        )
    )
    parent = "\n".join(
        (
            "import os, pathlib, subprocess, sys, time",
            f"subprocess.Popen([sys.executable, '-c', {child!r}])",
            f"p = pathlib.Path({str(pid_path)!r})",
            "with p.open('a', encoding='ascii') as stream:",
            "    stream.write(f'{os.getpid()}\\n')",
            "while True: time.sleep(1)",
        )
    )
    return (sys.executable, "-c", parent)


def _wait_for_tree(path: Path) -> None:
    deadline = time.monotonic() + 5
    while len(_read_pids(path)) < 3 and time.monotonic() < deadline:
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


def _wait_pid_stopped(pid: int) -> bool:
    deadline = time.monotonic() + 3
    while _pid_running(pid) and time.monotonic() < deadline:
        time.sleep(0.05)
    return not _pid_running(pid)


def _pid_running(pid: int) -> bool:
    stat = Path(f"/proc/{pid}/stat")
    if stat.exists():
        try:
            if stat.read_text(encoding="ascii").split()[2] == "Z":
                return False
        except (OSError, IndexError):
            pass
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True
