from __future__ import annotations

import subprocess
import unittest
from collections.abc import Callable
from unittest.mock import patch

import codex_desktop_bridge_macos_input as macos_input
import codex_desktop_bridge_macos_ui as macos_ui
from codex_thread_models import WindowInfo


class MacOSInputTests(unittest.TestCase):
    def test_window_snapshot_clipboard_and_keyboard_commands(self) -> None:
        calls: list[list[str]] = []

        def fake_run(args: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
            calls.append(args)
            if args[0] == "osascript" and "application processes" in " ".join(args):
                return subprocess.CompletedProcess(
                    args=args,
                    returncode=0,
                    stdout="123\tCodex\ttrue\t10\t20\t300\t400\tCodex - Thread\n",
                    stderr="",
                )
            if args[0] == "pbpaste":
                return subprocess.CompletedProcess(args=args, returncode=0, stdout="clip", stderr="")
            return subprocess.CompletedProcess(args=args, returncode=0, stdout="", stderr="")

        with patch.object(subprocess, "run", fake_run):
            seen: list[int] = []
            macos_input.enum_windows(lambda hwnd: seen.append(hwnd) or True)

            self.assertEqual(seen, [123001])
            self.assertEqual(macos_input.get_window_process_id(123001), 123)
            self.assertEqual(macos_input.get_window_text_length(123001), len("Codex - Thread"))
            self.assertEqual(macos_input.read_window_text(123001, 100), "Codex - Thread")
            self.assertEqual(macos_input.get_window_rect_tuple(123001), (10, 20, 310, 420))
            self.assertEqual(macos_input.get_foreground_window(), 123001)
            self.assertEqual(macos_input.get_clipboard_text(), "clip")

            macos_input.set_clipboard_text("hello")
            macos_input.send_hotkey(0x11, 0x12, 0x4C)
            macos_input.send_key_event(0x0D)
            self.assertEqual(
                macos_input.click_window(WindowInfo(hwnd=123001, title="Codex", left=10, top=20, right=310, bottom=420), 0.5, 50),
                (160, 70),
            )

        joined = "\n".join(" ".join(args) for args in calls)
        self.assertIn("pbcopy", joined)
        self.assertIn('keystroke "l" using {command down, option down}', joined)
        self.assertIn("key code 36", joined)
        self.assertIn("click at {160, 70}", joined)

    def test_runner_adapters_return_completed_processes(self) -> None:
        def fake_run(args: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
            script = " ".join(args)
            if "application processes" in script:
                return subprocess.CompletedProcess(
                    args=args,
                    returncode=0,
                    stdout="9\tCodex\ttrue\t1\t2\t300\t400\tCodex - Target\n",
                    stderr="",
                )
            if "entire contents" in script:
                return subprocess.CompletedProcess(args=args, returncode=0, stdout="Target row\n", stderr="")
            return subprocess.CompletedProcess(args=args, returncode=0, stdout="", stderr="")

        with patch.object(subprocess, "run", fake_run):
            self.assertEqual(macos_input.run_composer_focus_process(["ignored"]).stdout, "OK\n")
            self.assertEqual(
                macos_input.run_header_verification_process(["ignored"], env={"CODEX_THREAD_NAME": "Target"}).stdout,
                "OK:Codex - Target",
            )
            self.assertEqual(
                macos_input.run_sidebar_activation_process(
                    ["ignored"],
                    env={"CODEX_THREAD_NAME": "Target", "CODEX_PROJECT_NAME": "repo"},
                ).stdout,
                "OK:Target row",
            )
            self.assertEqual(
                macos_input.run_permission_approval_process(
                    ["ignored"],
                    env={"CODEX_APPROVAL_DECISION": "accept-remember"},
                ).stdout,
                "ACTION=accept-remember",
            )

    def test_ui_runner_keeps_default_backend_contract(self) -> None:
        def fake_run(args: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
            script = " ".join(args)
            if "application processes" in script:
                return subprocess.CompletedProcess(
                    args=args,
                    returncode=0,
                    stdout="9\tCodex\ttrue\t1\t2\t300\t400\tCodex - Direct\n",
                    stderr="",
                )
            return subprocess.CompletedProcess(args=args, returncode=0, stdout="", stderr="")

        with patch.object(subprocess, "run", fake_run):
            result = macos_ui.run_header_verification_process(
                ["ignored"],
                env={"CODEX_THREAD_NAME": "Direct"},
            )

        self.assertEqual(result.stdout, "OK:Codex - Direct")

    def test_ui_runner_rejects_invalid_backends_clearly(self) -> None:
        runner: Callable[..., subprocess.CompletedProcess[str]] = (
            macos_ui.run_header_verification_process
        )

        with self.assertRaisesRegex(RuntimeError, "macOS UI backend is unavailable"):
            _ = runner(["ignored"], backend=object())

        with (
            patch.object(macos_input, "MACOS_UI_BACKEND", object()),
            self.assertRaisesRegex(RuntimeError, "macOS UI backend is unavailable"),
        ):
            _ = macos_ui.run_header_verification_process(["ignored"])


if __name__ == "__main__":
    _ = unittest.main()
