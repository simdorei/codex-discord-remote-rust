# pyright: reportAny=false, reportAttributeAccessIssue=false, reportPrivateUsage=false, reportUnknownArgumentType=false, reportUnknownLambdaType=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnusedCallResult=false
from __future__ import annotations

import subprocess
import unittest
from typing import TypedDict, Unpack

import codex_desktop_bridge as bridge
import codex_desktop_bridge_window_focus as window_focus
from codex_thread_models import WindowInfo


class UnexpectedRunProcessError(Exception):
    pass


class RunProcessKwargs(TypedDict):
    capture_output: bool
    text: bool
    creationflags: int
    timeout: float
    check: bool


class WindowFocusTests(unittest.TestCase):
    def test_appx_chatgpt_window_is_recognized_as_codex_desktop(self) -> None:
        appx_path = (
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.727.6591.0_x64__2p2nqsd0c76g0"
            r"\app\ChatGPT.exe"
        )
        deps = _deps(
            windows=[7],
            titles={7: "ChatGPT"},
            rects={7: (10, 20, 810, 620)},
            process_paths={7: appx_path},
        )

        window = window_focus.find_codex_window(deps)

        self.assertEqual(window.hwnd, 7)
        self.assertEqual(window.title, "ChatGPT")

    def test_appx_chatgpt_window_rejects_lookalike_process_path(self) -> None:
        self.assertFalse(
            window_focus.is_codex_desktop_appx_window(
                "ChatGPT",
                r"C:\fake\OpenAI.Codex_fake\app\ChatGPT.exe",
            )
        )
        self.assertFalse(
            window_focus.is_codex_desktop_appx_window(
                "ChatGPT",
                r"C:\fake\WindowsApps\OpenAI.Codex_fake\app\ChatGPT.exe",
            )
        )

    def test_title_window_selection_focus_and_composer_retry(self) -> None:
        self.assertTrue(window_focus.is_codex_desktop_window_title(" Codex   -  Thread "))
        self.assertFalse(window_focus.is_codex_desktop_window_title("Other Codex"))

        calls: list[str] = []
        focus_results = iter(
            [
                subprocess.CompletedProcess(args=[], returncode=1, stdout="NO_PROSEMIRROR", stderr=""),
                subprocess.CompletedProcess(args=[], returncode=0, stdout="OK", stderr=""),
            ]
        )
        deps = _deps(
            windows=[1, 2, 3],
            visible={1: False, 2: True, 3: True},
            titles={2: "Other", 3: "Codex - Thread"},
            rects={3: (1, 2, 3, 4)},
            foreground=3,
            calls=calls,
            run=lambda args, **_kwargs: next(focus_results),
        )

        window = window_focus.find_codex_window(deps)
        self.assertEqual(window, WindowInfo(hwnd=3, title="Codex - Thread", left=1, top=2, right=3, bottom=4))

        window_focus.focus_window(window, deps)
        self.assertEqual(calls[:4], ["show:3", "set:3", "bring:3", "sleep:0.2"])

        self.assertTrue(window_focus.ensure_codex_composer_focus(2, deps))
        self.assertIn("key:9:down", calls)
        self.assertIn("key:9:up", calls)

    def test_bridge_window_text_and_title_wrappers_preserve_behavior(self) -> None:
        original_deps = bridge._make_window_text_deps
        try:
            bridge._make_window_text_deps = lambda: window_focus.WindowTextDeps(
                get_window_text_length=lambda _hwnd: 5,
                read_window_text=lambda _hwnd, _max_count: "Codex",
            )
            self.assertEqual(bridge.get_window_text(10), "Codex")
            self.assertTrue(bridge.is_codex_desktop_window_title("Codex - repo"))
        finally:
            bridge._make_window_text_deps = original_deps

    def test_no_visible_codex_window_and_composer_failures(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "Visible Codex Desktop window not found"):
            _ = window_focus.find_codex_window(
                _deps(windows=[1, 2], visible={1: False, 2: True}, titles={2: "Other"}, rects={})
            )

        self.assertFalse(
            window_focus.focus_codex_composer(
                _deps(run=lambda args, **_kwargs: subprocess.CompletedProcess(args=[], returncode=0, stdout="NO", stderr=""))
            )
        )

        def fail_run(args: list[str], **_kwargs: Unpack[RunProcessKwargs]) -> subprocess.CompletedProcess[str]:
            _ = args
            raise subprocess.TimeoutExpired(cmd=args, timeout=10.0)

        self.assertFalse(window_focus.focus_codex_composer(_deps(run=fail_run)))

        def unexpected_run(args: list[str], **_kwargs: Unpack[RunProcessKwargs]) -> subprocess.CompletedProcess[str]:
            _ = args
            raise UnexpectedRunProcessError("composer dependency broke")

        with self.assertRaisesRegex(UnexpectedRunProcessError, "composer dependency broke"):
            _ = window_focus.focus_codex_composer(_deps(run=unexpected_run))

    def test_retry_exhaustion_returns_false(self) -> None:
        calls: list[str] = []
        deps = _deps(
            calls=calls,
            run=lambda args, **_kwargs: subprocess.CompletedProcess(args=[], returncode=1, stdout="NO", stderr=""),
        )

        self.assertFalse(window_focus.ensure_codex_composer_focus(2, deps))
        self.assertEqual(calls.count("key:9:down"), 2)
        self.assertEqual(calls.count("key:9:up"), 2)


def _deps(
    *,
    windows: list[int] | None = None,
    visible: dict[int, bool] | None = None,
    titles: dict[int, str] | None = None,
    rects: dict[int, tuple[int, int, int, int]] | None = None,
    foreground: int = 0,
    process_paths: dict[int, str] | None = None,
    calls: list[str] | None = None,
    run: window_focus.RunProcess | None = None,
) -> window_focus.WindowFocusDeps:
    call_log = calls if calls is not None else []
    visible_map = visible or {}
    title_map = titles or {}
    rect_map = rects or {}
    process_path_map = process_paths or {}

    def enum_windows(callback: window_focus.WindowCallback) -> None:
        for hwnd in windows or []:
            if not callback(hwnd):
                break

    return window_focus.WindowFocusDeps(
        enum_windows=enum_windows,
        is_window_visible=lambda hwnd: visible_map.get(hwnd, True),
        get_window_text=lambda hwnd: title_map.get(hwnd, ""),
        get_window_process_path=lambda hwnd: process_path_map.get(hwnd, ""),
        get_window_rect=lambda hwnd: rect_map.get(hwnd),
        get_foreground_window=lambda: foreground,
        show_window=lambda hwnd: call_log.append(f"show:{hwnd}"),
        set_foreground_window=lambda hwnd: call_log.append(f"set:{hwnd}"),
        bring_window_to_top=lambda hwnd: call_log.append(f"bring:{hwnd}"),
        run_process=run
        or (lambda args, **_kwargs: subprocess.CompletedProcess(args=[], returncode=0, stdout="OK", stderr="")),
        send_key_event=lambda key, keyup: call_log.append(f"key:{key}:{'up' if keyup else 'down'}"),
        sleep=lambda seconds: call_log.append(f"sleep:{seconds}"),
        restore_command=9,
        tab_key=9,
    )


if __name__ == "__main__":
    unittest.main()
