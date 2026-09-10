from __future__ import annotations

# pyright: reportAny=false
import ctypes
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from collections.abc import Generator
from contextlib import contextmanager
from pathlib import Path
from typing import IO, cast

import codex_app_server_transport_process as process_mod

_CREATE_NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)
_PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
_STILL_ACTIVE = 259


def _pid_exists(pid: int) -> bool:
    if os.name != "nt":
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return False
        except PermissionError:
            return True
        return True
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


def _wait_pid_gone(pid: int, timeout_sec: float = 3.0) -> bool:
    deadline = time.monotonic() + timeout_sec
    while time.monotonic() < deadline:
        if not _pid_exists(pid):
            return True
        time.sleep(0.05)
    return not _pid_exists(pid)


def _kill_exact_tree(pid: int) -> None:
    if not _pid_exists(pid):
        return
    if os.name == "nt":
        _ = subprocess.run(
            ["taskkill.exe", "/PID", str(pid), "/T", "/F"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
            creationflags=_CREATE_NO_WINDOW,
        )
        return
    try:
        os.kill(pid, 9)
    except ProcessLookupError:
        return


@contextmanager
def _stubborn_process_tree(
    *,
    parent_exits_on_stdin: bool = False,
) -> Generator[tuple[process_mod.ResidentProcess, tuple[int, int, int]]]:
    grandchild_code = "import time; time.sleep(300)"
    child_code = "\n".join(
        (
            "import json, subprocess, sys, time",
            f"grandchild = subprocess.Popen([sys.executable, '-c', {grandchild_code!r}])",
            "print(json.dumps({'child': __import__('os').getpid(), 'grandchild': grandchild.pid}), flush=True)",
            "time.sleep(300)",
        )
    )
    parent_lines = [
        "import json, os, subprocess, sys, time",
        (
            "child = subprocess.Popen("
            f"[sys.executable, '-u', '-c', {child_code!r}], "
            "stdout=subprocess.PIPE, text=True)"
        ),
        "line = child.stdout.readline()",
        "payload = json.loads(line)",
        "print(json.dumps({'parent': os.getpid(), **payload}), flush=True)",
        "sys.stdin.read()" if parent_exits_on_stdin else "time.sleep(300)",
    ]
    parent_code = "\n".join(parent_lines)
    process = process_mod.start_owned_app_server_command(
        [sys.executable, "-u", "-c", parent_code]
    )
    stdout = cast(IO[str], process.stdout)
    payload = json.loads(stdout.readline())
    pids = (int(payload["parent"]), int(payload["child"]), int(payload["grandchild"]))
    try:
        yield process, pids
    finally:
        for pid in pids:
            _kill_exact_tree(pid)
        try:
            _ = process.wait(timeout=3.0)
        except subprocess.TimeoutExpired:
            process.kill()
            _ = process.wait(timeout=3.0)
        for pipe in (process.stdin, process.stdout, process.stderr):
            if pipe is not None and not pipe.closed:
                pipe.close()


@unittest.skipUnless(os.name == "nt", "Windows task-tree semantics")
class ResidentAppServerProcessTreeTests(unittest.TestCase):
    def test_close_removes_owned_descendants_but_preserves_unrelated_process(
        self,
    ) -> None:
        sentinel = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(300)"],
            creationflags=_CREATE_NO_WINDOW,
        )
        logs: list[str] = []
        try:
            with _stubborn_process_tree() as (process, pids):
                process_mod.close_resident_app_server_process(process, logs.append)

                self.assertIsNotNone(process.poll())
                self.assertTrue(all(_wait_pid_gone(pid) for pid in pids[1:]), pids)
                self.assertIsNone(sentinel.poll())
        finally:
            _kill_exact_tree(sentinel.pid)
            _ = sentinel.wait(timeout=3.0)

    def test_graceful_parent_exit_still_removes_preexisting_descendants(self) -> None:
        with _stubborn_process_tree(parent_exits_on_stdin=True) as (process, pids):
            process_mod.close_resident_app_server_process(process, lambda _line: None)

            self.assertIsNotNone(process.poll())
            self.assertTrue(all(_wait_pid_gone(pid) for pid in pids[1:]), pids)

    def test_job_ownership_removes_a_child_spawned_during_graceful_shutdown(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            pid_path = Path(temp_dir) / "late-child.pid"
            parent_code = "\n".join(
                (
                    "import pathlib, subprocess, sys",
                    "sys.stdin.read()",
                    "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(300)'])",
                    f"pathlib.Path({str(pid_path)!r}).write_text(str(child.pid), encoding='ascii')",
                )
            )
            process = process_mod.start_owned_app_server_command(
                [sys.executable, "-u", "-c", parent_code]
            )
            late_pid = 0
            try:
                process_mod.close_resident_app_server_process(
                    process, lambda _line: None
                )
                deadline = time.monotonic() + 3.0
                while time.monotonic() < deadline and not pid_path.exists():
                    time.sleep(0.02)
                late_pid = int(pid_path.read_text(encoding="ascii"))

                self.assertTrue(_wait_pid_gone(late_pid), late_pid)
            finally:
                if late_pid:
                    _kill_exact_tree(late_pid)


if __name__ == "__main__":
    _ = unittest.main()
