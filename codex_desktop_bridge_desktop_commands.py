from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from codex_thread_models import WindowInfo


class StartedProcess(Protocol):
    @property
    def pid(self) -> int: ...

    def poll(self) -> int | None: ...


ClickWindow = Callable[[WindowInfo, float, int], tuple[int, int]]
EnsureCodexDesktopExecutableConfigured = Callable[[], tuple[Path, str, bool]]
EnsureComposerFocus = Callable[[], bool]
FindCodexWindow = Callable[[], WindowInfo]
FocusWindow = Callable[[WindowInfo], None]
MakeConsoleSafeText = Callable[[str], str]
PrintLine = Callable[[str], None]
Sleep = Callable[[float], None]
StartCodexDesktopProcess = Callable[[Path], StartedProcess]
StopCodexDesktopProcesses = Callable[[Path], tuple[bool, str]]


@dataclass(frozen=True, slots=True)
class DesktopCommandDeps:
    ensure_codex_desktop_executable_configured: EnsureCodexDesktopExecutableConfigured
    stop_codex_desktop_processes: StopCodexDesktopProcesses
    start_codex_desktop_process: StartCodexDesktopProcess
    find_codex_window: FindCodexWindow
    focus_window: FocusWindow
    ensure_codex_composer_focus: EnsureComposerFocus
    click_window: ClickWindow
    make_console_safe_text: MakeConsoleSafeText
    sleep: Sleep
    print_line: PrintLine
    bridge_env_path: Path


class CodexDesktopImmediateExitError(RuntimeError):
    def __init__(self, exit_code: int) -> None:
        self.exit_code: int = exit_code
        super().__init__(f"Codex Desktop exited immediately after launch. exit_code={exit_code}")


def run_discover_codex_command(deps: DesktopCommandDeps) -> None:
    codex_exe, source, updated = deps.ensure_codex_desktop_executable_configured()
    deps.print_line(f"codex_desktop_exe: {codex_exe}")
    deps.print_line(f"source: {source}")
    deps.print_line(f"env_path: {deps.bridge_env_path}")
    deps.print_line(f"env_updated: {updated}")


def run_restart_codex_command(
    *,
    stop_wait: float,
    start_wait: float,
    deps: DesktopCommandDeps,
) -> None:
    codex_exe, source, updated = deps.ensure_codex_desktop_executable_configured()
    stopped, stop_details = deps.stop_codex_desktop_processes(codex_exe)
    deps.sleep(max(0.0, stop_wait))
    proc = deps.start_codex_desktop_process(codex_exe)
    deps.sleep(max(0.0, start_wait))
    exit_code = proc.poll()
    if exit_code is not None:
        raise CodexDesktopImmediateExitError(exit_code)
    deps.print_line(f"codex_desktop_exe: {codex_exe}")
    deps.print_line(f"source: {source}")
    deps.print_line(f"env_updated: {updated}")
    deps.print_line(f"stopped_existing: {stopped}")
    deps.print_line(f"stop_details: {deps.make_console_safe_text(stop_details)}")
    deps.print_line(f"started_pid: {proc.pid}")
    try:
        window = deps.find_codex_window()
        deps.print_line("window_found: True")
        deps.print_line(f"window_title: {deps.make_console_safe_text(window.title)}")
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK - CLI reports best-effort window lookup failure.
        deps.print_line("window_found: False")
        deps.print_line(f"window_error: {deps.make_console_safe_text(str(exc))}")


def run_focus_command(
    *,
    click: bool,
    click_x_ratio: float,
    click_y_offset: int,
    deps: DesktopCommandDeps,
) -> None:
    window = deps.find_codex_window()
    deps.focus_window(window)
    composer_focused = deps.ensure_codex_composer_focus()
    if click:
        x, y = deps.click_window(window, click_x_ratio, click_y_offset)
        deps.print_line(f"clicked: {x},{y}")
        composer_focused = deps.ensure_codex_composer_focus() or composer_focused
    rect_text = f"rect=({window.left},{window.top})-({window.right},{window.bottom})"
    deps.print_line(
        deps.make_console_safe_text(
            f"focused_window: hwnd={window.hwnd} title={window.title} {rect_text}"
        )
    )
    deps.print_line(f"composer_focused: {composer_focused}")
