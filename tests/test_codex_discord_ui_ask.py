from __future__ import annotations

import unittest

from codex_discord_ui_ask import build_ui_ask_argv, should_retry_ask_with_ui


class UiAskTests(unittest.TestCase):
    def test_should_retry_ask_with_ui_rejects_success(self) -> None:
        # Given
        output = "local sidecar could not attach"

        # When
        result = should_retry_ask_with_ui(0, output)

        # Then
        self.assertIs(result, False)

    def test_should_retry_ask_with_ui_accepts_known_attach_failures(self) -> None:
        for output in (
            "local sidecar could not attach",
            "ipc owner client for the selected thread was not discovered",
            "WinError 2",
            "WinError 5",
        ):
            with self.subTest(output=output):
                # Given / When
                result = should_retry_ask_with_ui(1, output)

                # Then
                self.assertIs(result, True)

    def test_build_ui_ask_argv_includes_thread_force_and_no_wait(self) -> None:
        # Given / When
        result = build_ui_ask_argv(
            "hello",
            target_thread_id="thread-1",
            force_while_busy=True,
            wait=False,
            timeout_sec=3.8,
        )

        # Then
        self.assertEqual(
            result,
            [
                "ask",
                "--ui",
                "--switch-thread",
                "--foreground",
                "--timeout",
                "3",
                "--thread-id",
                "thread-1",
                "--force-while-busy",
                "--no-wait",
                "hello",
            ],
        )

    def test_build_ui_ask_argv_defaults_timeout_to_zero(self) -> None:
        # Given / When
        result = build_ui_ask_argv(
            "hello",
            target_thread_id=None,
            force_while_busy=False,
            wait=True,
        )

        # Then
        self.assertEqual(
            result,
            ["ask", "--ui", "--switch-thread", "--foreground", "--timeout", "0", "hello"],
        )
