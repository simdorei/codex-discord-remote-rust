from __future__ import annotations

import subprocess
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from codex_thread_models import WindowInfo

ReadText = Callable[[Path], str]
FindCodexWindow = Callable[[], WindowInfo]
FocusWindow = Callable[[WindowInfo], None]
EnvironCopy = Callable[[], dict[str, str]]


class SidebarActivationError(RuntimeError):
    pass


class RunProcess(Protocol):
    def __call__(
        self,
        args: list[str],
        *,
        capture_output: bool,
        text: bool,
        encoding: str,
        errors: str,
        creationflags: int,
        timeout: float,
        check: bool,
        env: dict[str, str],
    ) -> subprocess.CompletedProcess[str]: ...


@dataclass(frozen=True, slots=True)
class SidebarActivationDeps:
    legacy_script_path: Path
    v2_script_path: Path
    read_text: ReadText
    find_codex_window: FindCodexWindow
    focus_window: FocusWindow
    run_process: RunProcess
    environ_copy: EnvironCopy
    create_no_window: int = 0


def legacy_activate_thread_by_sidebar(
    thread_name: str,
    project_name: str | None,
    deps: SidebarActivationDeps,
) -> str:
    return _activate_thread_by_sidebar(
        thread_name,
        project_name,
        script_path=deps.legacy_script_path,
        timeout_sec=15.0,
        focus_first=False,
        deps=deps,
    )


def activate_thread_by_sidebar_v2(
    thread_name: str,
    project_name: str | None,
    deps: SidebarActivationDeps,
) -> str:
    return _activate_thread_by_sidebar(
        thread_name,
        project_name,
        script_path=deps.v2_script_path,
        timeout_sec=25.0,
        focus_first=True,
        deps=deps,
    )


def _activate_thread_by_sidebar(
    thread_name: str,
    project_name: str | None,
    *,
    script_path: Path,
    timeout_sec: float,
    focus_first: bool,
    deps: SidebarActivationDeps,
) -> str:
    if not thread_name.strip():
        raise SidebarActivationError("Missing thread_name for sidebar activation.")

    window: WindowInfo | None = None
    if focus_first:
        window = deps.find_codex_window()
        deps.focus_window(window)

    script = deps.read_text(script_path)
    env = deps.environ_copy()
    env["CODEX_THREAD_NAME"] = thread_name
    env["CODEX_PROJECT_NAME"] = project_name or ""
    env["CODEX_DESKTOP_BRIDGE_SCRIPT_DIR"] = str(script_path.parent)
    if window is not None:
        env["CODEX_WINDOW_HANDLE"] = str(window.hwnd)

    try:
        result = deps.run_process(
            ["powershell", "-NoProfile", "-Command", script],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            creationflags=deps.create_no_window,
            timeout=timeout_sec,
            check=False,
            env=env,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        raise SidebarActivationError(f"Sidebar activation subprocess failed: {exc}") from exc

    output = (result.stdout or "").strip()
    error = (result.stderr or "").strip()
    if result.returncode != 0 or not output.startswith("OK:"):
        detail = output or error or f"exit={result.returncode}"
        raise SidebarActivationError(f"Sidebar activation failed: {detail}")
    return output[3:].strip()
