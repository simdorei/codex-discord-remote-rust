# pyright: reportAny=false, reportAttributeAccessIssue=false, reportPrivateLocalImportUsage=false, reportPrivateUsage=false, reportUnknownArgumentType=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnusedCallResult=false
from __future__ import annotations

import json
import subprocess
import time
import unittest
from collections.abc import Iterable

import codex_desktop_bridge as bridge
import codex_desktop_bridge_sidecar as sidecar
import codex_desktop_bridge_sidecar_thread as sidecar_thread
from codex_desktop_bridge_sidecar_protocol import CodexSidecarProtocolError


class SidecarHappyTests(unittest.TestCase):
    def test_happy_path_frames_requests_and_preserves_bridge_exports(self) -> None:
        process = FakeProcess(
            [
                _result("1", {}),
                _result("2", {"thread": {"id": "thread-1"}}),
                _result("3", {"thread": {"id": "thread-1", "status": {"type": "idle"}}}),
                _result("4", {}),
                _result("5", {"data": [{"model": "gpt-5.5"}]}),
                _result("6", {"turn": {"id": "turn-1"}}),
                _result("7", {}),
                _result("8", {}),
            ]
        )

        client = sidecar.CodexAppServerSidecar(
            executable_resolver=lambda: "codex-test",
            start_process=FakeStarter(process),
        )
        self.assertIs(bridge.CodexAppServerSidecar, sidecar.CodexAppServerSidecar)
        self.assertIs(bridge.CodexSidecarError, sidecar.CodexSidecarError)
        self.assertEqual(client.read_thread("thread-1", include_turns=True)["thread"], {"id": "thread-1"})
        self.assertEqual(client.resume_thread("thread-1")["thread"], {"id": "thread-1", "status": {"type": "idle"}})
        self.assertEqual(client.interrupt_turn("thread-1", "turn-1"), {})
        self.assertEqual(client.list_models(), {"data": [{"model": "gpt-5.5"}]})
        self.assertEqual(client.start_turn("thread-1", "hello"), {"turn": {"id": "turn-1"}})
        self.assertEqual(client.archive_thread("thread-1"), {})
        self.assertEqual(client.update_thread_settings("thread-1", {"model": "gpt-5.5", "effort": "high"}), {})
        client.close()
        client.close()

        payloads = [json.loads(raw) for raw in _require_stdin(process).lines]
        self.assertEqual([payload["id"] for payload in payloads], [str(index) for index in range(1, 9)])
        self.assertEqual(
            [payload["method"] for payload in payloads],
            [
                "initialize",
                "thread/read",
                "thread/resume",
                "turn/interrupt",
                "model/list",
                "turn/start",
                "thread/archive",
                "thread/settings/update",
            ],
        )
        self.assertEqual(payloads[1]["params"], {"threadId": "thread-1", "includeTurns": True})
        self.assertEqual(payloads[5]["params"]["input"][0]["text"], "hello")
        self.assertEqual(payloads[7]["params"]["effort"], "high")
        self.assertTrue(process.terminated)


class SidecarEdgeTests(unittest.TestCase):
    def test_startup_response_close_and_timeout_failures_are_surfaced(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "Failed to start"):
            _ = sidecar.CodexAppServerSidecar(
                executable_resolver=lambda: "missing",
                start_process=FailingStarter(),
            )

        with self.assertRaisesRegex(RuntimeError, "Failed to start"):
            _ = sidecar.CodexAppServerSidecar(
                executable_resolver=lambda: "broken-stdio",
                start_process=FakeStarter(FakeProcess([_result("1", {})], missing_stdin=True)),
            )

        client = _client([_result("1", {}), "not-json"])
        with self.assertRaisesRegex(RuntimeError, "non-JSON"):
            _ = client.request("model/list", {}, timeout_sec=0.1)
        client.close()

        client = _client([_result("1", {}), json.dumps({"id": "2", "error": {"message": "boom"}})])
        with self.assertRaisesRegex(sidecar.CodexSidecarError, "boom"):
            _ = client.request("model/list", {}, timeout_sec=0.1)
        client.close()

        client = _client([_result("1", {}), json.dumps({"id": "2"})])
        with self.assertRaisesRegex(RuntimeError, "invalid payload"):
            _ = client.request("model/list", {}, timeout_sec=0.1)
        client.close()

        client = _client([_result("1", {}), _result("2", {"thread": []})])
        with self.assertRaisesRegex(CodexSidecarProtocolError, "invalid thread payload"):
            _ = client.read_thread("thread-1")
        client.close()

        with self.assertRaisesRegex(CodexSidecarProtocolError, "thread/resume did not return a thread payload"):
            _ = sidecar_thread.ensure_thread_loaded_via_sidecar(InvalidResumePayloadClient(), "thread-1")

        exited = sidecar.CodexAppServerSidecar(
            executable_resolver=lambda: "codex-test",
            start_process=FakeStarter(FakeProcess([])),
            initialize=False,
        )
        with self.assertRaisesRegex(RuntimeError, "exited while waiting"):
            _ = exited.request("model/list", {}, timeout_sec=0.1)
        exited.close()

        timeout = sidecar.CodexAppServerSidecar(
            executable_resolver=lambda: "codex-test",
            start_process=FakeStarter(FakeProcess(BlockingStdout())),
            initialize=False,
        )
        with self.assertRaises(TimeoutError):
            _ = timeout.request("model/list", {}, timeout_sec=0.01)
        timeout.close()

        process = FakeProcess([], wait_timeout=True)
        client = sidecar.CodexAppServerSidecar(
            executable_resolver=lambda: "codex-test",
            start_process=FakeStarter(process),
            initialize=False,
        )
        client.close()
        client.close()
        self.assertTrue(process.terminated)
        self.assertTrue(process.killed)


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
    def __init__(
        self,
        stdout: Iterable[str] | None,
        *,
        stdin: FakeStdin | None = None,
        missing_stdin: bool = False,
        wait_timeout: bool = False,
    ) -> None:
        self.stdin: FakeStdin | None = None if missing_stdin else (stdin or FakeStdin())
        self.stdout: Iterable[str] | None = stdout
        self.returncode: int | None = None
        self.terminated: bool = False
        self.killed: bool = False
        self.wait_timeout: bool = wait_timeout

    def poll(self) -> int | None:
        return self.returncode

    def terminate(self) -> None:
        self.terminated = True
        if not self.wait_timeout:
            self.returncode = 0

    def wait(self, timeout: float | None = None) -> int:
        _ = timeout
        if self.wait_timeout:
            raise subprocess.TimeoutExpired("codex", 1.5)
        return self.returncode or 0

    def kill(self) -> None:
        self.killed = True
        self.returncode = -9


class FakeStarter:
    def __init__(self, process: FakeProcess) -> None:
        self.process: FakeProcess = process

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


class FailingStarter:
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
        raise OSError("missing executable")


class InvalidResumePayloadClient:
    def read_thread(self, thread_id: str, *, include_turns: bool = False) -> sidecar.JsonObject:
        _ = (thread_id, include_turns)
        return {"thread": {"status": {"type": "notLoaded"}}}

    def resume_thread(self, thread_id: str) -> sidecar.JsonObject:
        _ = thread_id
        return {}


class BlockingStdout:
    def __iter__(self) -> "BlockingStdout":
        return self

    def __next__(self) -> str:
        time.sleep(0.2)
        raise StopIteration


def _client(lines: list[str]) -> sidecar.CodexAppServerSidecar:
    return sidecar.CodexAppServerSidecar(
        executable_resolver=lambda: "codex-test",
        start_process=FakeStarter(FakeProcess(lines)),
    )


def _result(request_id: str, result: sidecar.JsonObject) -> str:
    return json.dumps({"id": request_id, "result": result})


def _require_stdin(process: FakeProcess) -> FakeStdin:
    stdin = process.stdin
    if stdin is None:
        raise AssertionError("expected fake stdin")
    return stdin


if __name__ == "__main__":
    unittest.main()
