from __future__ import annotations

import unittest
from dataclasses import dataclass
from pathlib import Path

import codex_desktop_bridge_desktop_commands as desktop_commands
from codex_thread_models import WindowInfo


class DesktopCommandTests(unittest.TestCase):
    def test_run_restart_codex_command_raises_typed_error_when_process_exits_immediately(self) -> None:
        sleeps: list[float] = []
        printed_lines: list[str] = []
        deps = _deps(
            start_codex_desktop_process=lambda _path: FakeStartedProcess(pid=1234, exit_code=9),
            sleep=sleeps.append,
            print_line=printed_lines.append,
        )

        with self.assertRaisesRegex(
            desktop_commands.CodexDesktopImmediateExitError,
            "Codex Desktop exited immediately after launch. exit_code=9",
        ):
            desktop_commands.run_restart_codex_command(
                stop_wait=0.25,
                start_wait=0.5,
                deps=deps,
            )

        self.assertEqual(sleeps, [0.25, 0.5])
        self.assertEqual(printed_lines, [])


@dataclass(frozen=True, slots=True)
class FakeStartedProcess:
    pid: int
    exit_code: int | None

    def poll(self) -> int | None:
        return self.exit_code


def _deps(
    *,
    start_codex_desktop_process: desktop_commands.StartCodexDesktopProcess | None = None,
    sleep: desktop_commands.Sleep | None = None,
    print_line: desktop_commands.PrintLine | None = None,
) -> desktop_commands.DesktopCommandDeps:
    return desktop_commands.DesktopCommandDeps(
        ensure_codex_desktop_executable_configured=_ensure_desktop_exe,
        stop_codex_desktop_processes=lambda _path: (True, "stopped"),
        start_codex_desktop_process=start_codex_desktop_process or _start_process,
        find_codex_window=_find_window,
        focus_window=lambda _window: None,
        ensure_codex_composer_focus=lambda: True,
        click_window=lambda _window, _x_ratio, _y_offset: (10, 20),
        make_console_safe_text=lambda text: text,
        sleep=sleep or _ignore_sleep,
        print_line=print_line or _ignore_line,
        bridge_env_path=Path(".env"),
    )


def _ensure_desktop_exe() -> tuple[Path, str, bool]:
    return (Path("C:/Codex/Codex.exe"), "test", False)


def _start_process(_path: Path) -> FakeStartedProcess:
    return FakeStartedProcess(pid=1234, exit_code=None)


def _find_window() -> WindowInfo:
    return WindowInfo(hwnd=1, title="Codex", left=0, top=0, right=100, bottom=80)


def _ignore_sleep(_seconds: float) -> None:
    return None


def _ignore_line(_line: str) -> None:
    return None


if __name__ == "__main__":
    _ = unittest.main()
