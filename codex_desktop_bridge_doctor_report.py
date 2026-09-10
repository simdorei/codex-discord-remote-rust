from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

from codex_thread_models import ThreadContextUsage, ThreadInfo, WindowInfo


ActiveThreadCount = Callable[[], tuple[int, str]]
DiscoverDesktopExecutable = Callable[[], tuple[Path | None, str]]
FindCodexWindow = Callable[[], WindowInfo]
GetBusyThreads = Callable[[int], list[ThreadInfo]]
GetHighContextThreads = Callable[[int], list[tuple[ThreadInfo, ThreadContextUsage]]]
GetSelectedThreadId = Callable[[], str | None]
GetThreadWorkspaceRef = Callable[[ThreadInfo], str]
IsProtocolRegistered = Callable[[str], bool]
MakeConsoleSafeText = Callable[[str], str]
PrintLine = Callable[[str], None]


@dataclass(frozen=True, slots=True)
class DoctorReportConfig:
    platform_text: str
    python_version: str
    python_executable: str
    codex_home: Path
    state_db_path: Path
    session_index_path: Path
    global_state_path: Path
    bridge_state_path: Path
    high_context_input_ratio_threshold: float


@dataclass(frozen=True, slots=True)
class CodexAppUpdateStatus:
    current_version: str | None
    previous_version: str | None
    update_detected: bool
    details: str


CheckCodexAppUpdate = Callable[[], CodexAppUpdateStatus]


@dataclass(frozen=True, slots=True)
class DoctorReportDeps:
    is_protocol_registered: IsProtocolRegistered
    get_selected_thread_id: GetSelectedThreadId
    active_thread_count: ActiveThreadCount
    get_high_context_threads: GetHighContextThreads
    get_thread_workspace_ref: GetThreadWorkspaceRef
    find_codex_window: FindCodexWindow
    make_console_safe_text: MakeConsoleSafeText
    get_busy_threads: GetBusyThreads
    discover_codex_desktop_executable: DiscoverDesktopExecutable
    check_codex_app_update: CheckCodexAppUpdate
    print_line: PrintLine


def print_doctor_report(limit: int, config: DoctorReportConfig, deps: DoctorReportDeps) -> None:
    _print_static_status(config, deps)
    thread_count, db_error = deps.active_thread_count()
    deps.print_line(f"thread_count: {thread_count}")
    if db_error:
        deps.print_line(f"db_error: {db_error}")

    deps.print_line(f"high_context_threshold: {config.high_context_input_ratio_threshold * 100:.1f}%")
    _print_high_context_threads(limit, deps)
    _print_codex_window(deps)
    _print_busy_threads(limit, deps)
    desktop_exe, desktop_source = deps.discover_codex_desktop_executable()
    deps.print_line(f"codex_desktop_exe: {desktop_exe or '-'}")
    deps.print_line(f"codex_desktop_exe_source: {desktop_source or '-'}")
    _print_codex_app_update_status(deps)


def _print_static_status(config: DoctorReportConfig, deps: DoctorReportDeps) -> None:
    deps.print_line(f"platform: {config.platform_text}")
    deps.print_line(f"python_version: {config.python_version}")
    deps.print_line(f"python_executable: {config.python_executable}")
    deps.print_line(f"codex_home: {config.codex_home}")
    deps.print_line(f"codex_home_exists: {config.codex_home.exists()}")
    deps.print_line(f"state_db_path: {config.state_db_path}")
    deps.print_line(f"state_db_exists: {config.state_db_path.exists()}")
    deps.print_line(f"session_index_path: {config.session_index_path}")
    deps.print_line(f"session_index_exists: {config.session_index_path.exists()}")
    deps.print_line(f"global_state_path: {config.global_state_path}")
    deps.print_line(f"global_state_exists: {config.global_state_path.exists()}")
    deps.print_line(f"bridge_state_path: {config.bridge_state_path}")
    deps.print_line(f"bridge_state_parent_exists: {config.bridge_state_path.parent.exists()}")
    deps.print_line(f"codex_protocol_registered: {deps.is_protocol_registered('codex')}")
    deps.print_line(f"selected_thread_id: {deps.get_selected_thread_id() or '-'}")


def _print_high_context_threads(limit: int, deps: DoctorReportDeps) -> None:
    scan_limit = max(20, limit * 4)
    high_context_threads = deps.get_high_context_threads(scan_limit)
    if not high_context_threads:
        deps.print_line("high_context_threads: -")
        return
    labels = ", ".join(
        f"{deps.get_thread_workspace_ref(thread)}={usage.usage_ratio * 100:.1f}%"
        for thread, usage in high_context_threads[:limit]
    )
    deps.print_line(f"high_context_threads: {labels}")


def _print_codex_window(deps: DoctorReportDeps) -> None:
    try:
        window = deps.find_codex_window()
        rect = f"({window.left},{window.top})-({window.right},{window.bottom})"
        deps.print_line("codex_window_found: True")
        deps.print_line(f"codex_window_title: {deps.make_console_safe_text(window.title)}")
        deps.print_line(deps.make_console_safe_text(f"codex_window_rect: {rect}"))
    except RuntimeError as exc:
        deps.print_line("codex_window_found: False")
        deps.print_line(f"codex_window_error: {deps.make_console_safe_text(str(exc))}")


def _print_busy_threads(limit: int, deps: DoctorReportDeps) -> None:
    busy_threads = deps.get_busy_threads(max(10, limit))
    if not busy_threads:
        deps.print_line("busy_threads: -")
        return
    labels = ", ".join(deps.get_thread_workspace_ref(thread) for thread in busy_threads[:limit])
    deps.print_line(f"busy_threads: {labels}")


def _print_codex_app_update_status(deps: DoctorReportDeps) -> None:
    status = deps.check_codex_app_update()
    deps.print_line(f"codex_app_package_version: {status.current_version or '-'}")
    deps.print_line(f"codex_app_previous_package_version: {status.previous_version or '-'}")
    deps.print_line(f"codex_app_update_detected: {status.update_detected}")
    deps.print_line(f"codex_app_restart_recommended: {status.update_detected}")
    if status.details:
        deps.print_line(f"codex_app_package_version_status: {deps.make_console_safe_text(status.details)}")
