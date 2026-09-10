# pyright: reportAny=false, reportAttributeAccessIssue=false, reportPrivateUsage=false, reportUnknownArgumentType=false, reportUnknownLambdaType=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnusedCallResult=false
from __future__ import annotations

import subprocess
import unittest
from dataclasses import dataclass

import codex_desktop_bridge as bridge
import codex_desktop_bridge_active_thread as active_thread
from codex_thread_models import WindowInfo


class ClipboardRestoreError(RuntimeError):
    pass


class UnexpectedClipboardRestoreError(Exception):
    pass


class UnexpectedHeaderRunError(Exception):
    pass


RunKwargValue = bool | int | float | str | dict[str, str]
RunCallValue = list[str] | RunKwargValue
ClipboardCall = tuple[str, tuple[int, ...]] | tuple[str, int] | tuple[str, int, bool] | tuple[str, float]


class ActiveThreadHappyTests(unittest.TestCase):
    def test_clipboard_verification_header_parsing_and_bridge_wrappers(self) -> None:
        deeplink = _clipboard_harness(["original", "codex://thread/thread-1"])
        self.assertEqual(active_thread.verify_active_thread("thread-1", deeplink.deps), "copy-deeplink")
        self.assertEqual(deeplink.set_values[-1], "original")
        self.assertIn(("hotkey", (deeplink.vk_control, deeplink.vk_menu, deeplink.vk_l)), deeplink.calls)

        session = _clipboard_harness(["original", "unchanged-l", "thread-1"])
        self.assertEqual(active_thread.verify_active_thread("thread-1", session.deps), "copy-session-id")
        self.assertEqual(session.set_values[-1], "original")
        self.assertIn(("hotkey", (session.vk_control, session.vk_menu, session.vk_c)), session.calls)

        run_calls: list[dict[str, RunCallValue]] = []

        def run_process(args: list[str], **kwargs: RunKwargValue) -> subprocess.CompletedProcess[str]:
            run_calls.append({"args": args, **kwargs})
            return subprocess.CompletedProcess(args=args, returncode=0, stdout="OK:Thread", stderr="")

        header = active_thread.verify_active_thread_by_header("Thread", _clipboard_harness(["original"], run=run_process).deps)
        self.assertEqual(header, "header")
        self.assertEqual(
            run_calls[0]["env"],
            {"CODEX_THREAD_NAME": "Thread", "CODEX_WINDOW_HANDLE": "1"},
        )

        original_deps = bridge._make_active_thread_deps
        try:
            bridge._make_active_thread_deps = lambda: _clipboard_harness(["original", "codex://thread/thread-1"]).deps
            self.assertEqual(bridge.verify_active_thread("thread-1"), "copy-deeplink")
        finally:
            bridge._make_active_thread_deps = original_deps


class ActiveThreadEdgeTests(unittest.TestCase):
    def test_header_and_clipboard_failure_edges_restore_clipboard(self) -> None:
        run_calls: list[str] = []
        deps = _clipboard_harness(
            ["original"],
            run=lambda args, **kwargs: run_calls.append("run") or _completed(args, 5, "NO_HEADER"),
        ).deps

        self.assertIsNone(active_thread.verify_active_thread_by_header("   ", deps))
        self.assertEqual(run_calls, [])
        self.assertIsNone(active_thread.verify_active_thread_by_header("Thread", deps))

        def failing_run(args: list[str], **kwargs: RunKwargValue) -> subprocess.CompletedProcess[str]:
            _ = (args, kwargs)
            raise subprocess.SubprocessError("boom")

        self.assertIsNone(
            active_thread.verify_active_thread_by_header("Thread", _clipboard_harness(["original"], run=failing_run).deps)
        )

        failed = _clipboard_harness(["original", "sentinel", "sentinel", "sentinel", "sentinel"])
        self.assertIsNone(active_thread.verify_active_thread("thread-1", failed.deps))
        self.assertEqual(failed.set_values[-1], "original")
        self.assertIn(("key", failed.vk_escape, False), failed.calls)
        self.assertIn(("key", failed.vk_escape, True), failed.calls)

    def test_expected_clipboard_restore_failure_preserves_result(self) -> None:
        harness = _clipboard_harness(
            ["original", "codex://thread/thread-1"],
            restore_error=ClipboardRestoreError("clipboard locked"),
        )

        self.assertEqual(active_thread.verify_active_thread("thread-1", harness.deps), "copy-deeplink")

    def test_unexpected_clipboard_restore_failure_propagates(self) -> None:
        harness = _clipboard_harness(
            ["original", "codex://thread/thread-1"],
            restore_error=UnexpectedClipboardRestoreError("restore dependency broke"),
        )

        with self.assertRaisesRegex(UnexpectedClipboardRestoreError, "restore dependency broke"):
            active_thread.verify_active_thread("thread-1", harness.deps)

    def test_unexpected_header_process_failure_propagates(self) -> None:
        def run_process(args: list[str], **kwargs: RunKwargValue) -> subprocess.CompletedProcess[str]:
            _ = (args, kwargs)
            raise UnexpectedHeaderRunError("run dependency broke")

        with self.assertRaisesRegex(UnexpectedHeaderRunError, "run dependency broke"):
            active_thread.verify_active_thread_by_header(
                "Thread",
                _clipboard_harness(["original"], run=run_process).deps,
            )


def _completed(args: list[str], returncode: int, stdout: str) -> subprocess.CompletedProcess[str]:
    return subprocess.CompletedProcess(args=args, returncode=returncode, stdout=stdout, stderr="")


@dataclass(frozen=True, slots=True)
class ClipboardHarness:
    deps: active_thread.ActiveThreadDeps
    calls: list[ClipboardCall]
    set_values: list[str]
    vk_control: int
    vk_menu: int
    vk_l: int
    vk_c: int
    vk_escape: int


def _clipboard_harness(
    reads: list[str | None],
    *,
    run: active_thread.RunProcess | None = None,
    restore_error: Exception | None = None,
) -> ClipboardHarness:
    read_values = list(reads)
    calls: list[ClipboardCall] = []
    set_values: list[str] = []
    time_values = iter([1, 2, 3, 4, 5, 6])

    def get_clipboard_text() -> str | None:
        if read_values:
            return read_values.pop(0)
        return set_values[-1] if set_values else None

    def set_clipboard_text(text: str) -> None:
        if text == "original" and restore_error is not None:
            raise restore_error
        set_values.append(text)

    def send_hotkey(*keys: int) -> None:
        calls.append(("hotkey", keys))

    def send_key_event(vk: int, keyup: bool = False) -> None:
        calls.append(("key", vk, keyup))

    vk_control = 17
    vk_menu = 18
    vk_l = 76
    vk_c = 67
    vk_escape = 27
    deps = active_thread.ActiveThreadDeps(
        get_clipboard_text=get_clipboard_text,
        set_clipboard_text=set_clipboard_text,
        find_codex_window=lambda: WindowInfo(hwnd=1, title="Codex", left=0, top=0, right=100, bottom=100),
        focus_window=lambda window: calls.append(("focus", window.hwnd)),
        send_hotkey=send_hotkey,
        send_key_event=send_key_event,
        sleep=lambda seconds: calls.append(("sleep", seconds)),
        time_ns=lambda: next(time_values),
        run_process=run or (lambda args, **kwargs: _completed(args, 0, "OK:Thread")),
        environ_copy=lambda: {},
        vk_control=vk_control,
        vk_menu=vk_menu,
        vk_l=vk_l,
        vk_c=vk_c,
        vk_escape=vk_escape,
        create_no_window=0,
    )
    return ClipboardHarness(
        deps=deps,
        calls=calls,
        set_values=set_values,
        vk_control=vk_control,
        vk_menu=vk_menu,
        vk_l=vk_l,
        vk_c=vk_c,
        vk_escape=vk_escape,
    )


if __name__ == "__main__":
    unittest.main()
