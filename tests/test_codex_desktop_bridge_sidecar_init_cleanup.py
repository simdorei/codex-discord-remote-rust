# pyright: reportAny=false, reportAttributeAccessIssue=false, reportUnknownArgumentType=false, reportUnknownMemberType=false
from __future__ import annotations

import subprocess
import unittest
from collections.abc import Iterable

import codex_desktop_bridge_sidecar as sidecar


class SidecarInitCleanupTests(unittest.TestCase):
    def test_closes_started_process_when_initialize_times_out(self) -> None:
        process = FakeProcess([])
        original_reader = sidecar.sidecar_io.read_sidecar_response

        def raise_initialize_timeout(
            stdout_queue: sidecar.sidecar_io.ResponseQueue,
            request_id: str,
            method: str,
            timeout_sec: float,
        ) -> sidecar.JsonObject:
            _ = (stdout_queue, request_id, timeout_sec)
            raise TimeoutError(f"Timed out waiting for app-server response to {method}.")

        sidecar.sidecar_io.read_sidecar_response = raise_initialize_timeout
        try:
            with self.assertRaisesRegex(TimeoutError, "initialize"):
                _ = sidecar.CodexAppServerSidecar(
                    executable_resolver=lambda: "codex-test",
                    start_process=FakeStarter(process),
                )
        finally:
            sidecar.sidecar_io.read_sidecar_response = original_reader

        self.assertTrue(process.terminated)


class FakeStdin:
    def __init__(self) -> None:
        self.lines: list[str] = []
        self.closed: bool = False

    def write(self, value: str) -> int:
        self.lines.append(value.strip())
        return len(value)

    def flush(self) -> None:
        return None

    def close(self) -> None:
        self.closed = True


class FakeProcess:
    def __init__(self, stdout: Iterable[str] | None) -> None:
        self.stdin: FakeStdin | None = FakeStdin()
        self.stdout: Iterable[str] | None = stdout
        self.returncode: int | None = None
        self.terminated: bool = False
        self.killed: bool = False

    def poll(self) -> int | None:
        return self.returncode

    def terminate(self) -> None:
        self.terminated = True
        self.returncode = 0

    def wait(self, timeout: float | None = None) -> int:
        _ = timeout
        return self.returncode or 0

    def kill(self) -> None:
        self.killed = True
        self.returncode = -9


class FakeStarter:
    def __init__(self, process: FakeProcess) -> None:
        self.process = process

    def __call__(
        self,
        args: list[str],
        *,
        stdin: int,
        stdout: int,
        stderr: int,
        text: bool,
        encoding: str,
        bufsize: int,
        creationflags: int,
    ) -> sidecar.SidecarProcess:
        _ = (args, stdin, stdout, stderr, text, encoding, bufsize, creationflags)
        return self.process


if __name__ == "__main__":
    unittest.main()
