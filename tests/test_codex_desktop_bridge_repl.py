from __future__ import annotations

import argparse
import unittest
from collections import deque
from collections.abc import Callable

import codex_desktop_bridge_repl as repl


CommandFunc = Callable[[argparse.Namespace], int | None]


class ReplCommandError(RuntimeError):
    pass


class ReplTests(unittest.TestCase):
    def test_run_repl_surfaces_command_error_and_continues(self) -> None:
        printed: list[str] = []

        def command(_args: argparse.Namespace) -> int | None:
            raise ReplCommandError("boom")

        result = repl.run_repl(_deps(["status", "exit"], printed, command))

        self.assertEqual(result, 0)
        self.assertIn("ERROR: boom", printed)
        error_index = printed.index("ERROR: boom")
        self.assertLess(error_index + 1, len(printed))
        self.assertEqual(printed[error_index + 1], "")

    def test_run_repl_reports_invalid_command_and_continues(self) -> None:
        printed: list[str] = []

        result = repl.run_repl(_deps(["--bad", "exit"], printed, _ok_command))

        self.assertEqual(result, 0)
        self.assertIn("(invalid command)", printed)
        invalid_index = printed.index("(invalid command)")
        self.assertLess(invalid_index + 1, len(printed))
        self.assertEqual(printed[invalid_index + 1], "")

    def test_run_repl_reports_keyboard_interrupt_and_continues(self) -> None:
        printed: list[str] = []

        result = repl.run_repl(_deps(["doctor", "exit"], printed, _ok_command))

        self.assertEqual(result, 0)
        self.assertIn("Interrupted.", printed)
        interrupt_index = printed.index("Interrupted.")
        self.assertLess(interrupt_index + 1, len(printed))
        self.assertEqual(printed[interrupt_index + 1], "")


def _deps(inputs: list[str], printed: list[str], command_func: CommandFunc) -> repl.ReplDeps:
    input_queue: deque[str] = deque(inputs)

    def input_line(_prompt: str) -> str:
        if input_queue:
            return input_queue.popleft()
        raise EOFError

    return repl.ReplDeps(
        get_selected_thread_id=lambda: None,
        build_parser=lambda: _build_parser(command_func),
        split_repl_command=str.split,
        input_line=input_line,
        print_line=printed.append,
    )


def _build_parser(command_func: CommandFunc) -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="bridge")
    subparsers = parser.add_subparsers(dest="command", required=True)
    status_parser = subparsers.add_parser("status")
    status_parser.set_defaults(func=command_func)
    doctor_parser = subparsers.add_parser("doctor")
    doctor_parser.set_defaults(func=_interrupt_command)
    return parser


def _ok_command(_args: argparse.Namespace) -> int | None:
    return 0


def _interrupt_command(_args: argparse.Namespace) -> int | None:
    raise KeyboardInterrupt


if __name__ == "__main__":
    _ = unittest.main()
