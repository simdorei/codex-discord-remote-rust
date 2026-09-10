from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import codex_desktop_bridge_doctor_report as doctor
from codex_thread_models import ThreadContextUsage, ThreadInfo, WindowInfo


class WindowMissingError(RuntimeError):
    pass


class UnexpectedWindowProbeError(Exception):
    pass


def _config(root: Path) -> doctor.DoctorReportConfig:
    return doctor.DoctorReportConfig(
        platform_text="Windows",
        python_version="3.x",
        python_executable="python.exe",
        codex_home=root / "codex-home",
        state_db_path=root / "state.db",
        session_index_path=root / "sessions.json",
        global_state_path=root / "global.json",
        bridge_state_path=root / "bridge" / "state.json",
        high_context_input_ratio_threshold=0.9,
    )


def _is_protocol_registered(scheme: str) -> bool:
    _ = scheme
    return False


def _selected_thread_id() -> str | None:
    return None


def _active_thread_count() -> tuple[int, str]:
    return 0, ""


def _high_context_threads(limit: int) -> list[tuple[ThreadInfo, ThreadContextUsage]]:
    _ = limit
    return []


def _thread_workspace_ref(thread: ThreadInfo) -> str:
    return thread.id


def _safe_text(text: str) -> str:
    return text


def _busy_threads(limit: int) -> list[ThreadInfo]:
    _ = limit
    return []


def _desktop_executable() -> tuple[Path | None, str]:
    return None, ""


def _codex_app_update_status() -> doctor.CodexAppUpdateStatus:
    return doctor.CodexAppUpdateStatus(
        current_version="26.612.1.0",
        previous_version="26.611.8604.0",
        update_detected=True,
        details="Get-AppxPackage OpenAI.Codex",
    )


def _deps(lines: list[str], find_window: doctor.FindCodexWindow) -> doctor.DoctorReportDeps:
    return doctor.DoctorReportDeps(
        is_protocol_registered=_is_protocol_registered,
        get_selected_thread_id=_selected_thread_id,
        active_thread_count=_active_thread_count,
        get_high_context_threads=_high_context_threads,
        get_thread_workspace_ref=_thread_workspace_ref,
        find_codex_window=find_window,
        make_console_safe_text=_safe_text,
        get_busy_threads=_busy_threads,
        discover_codex_desktop_executable=_desktop_executable,
        check_codex_app_update=_codex_app_update_status,
        print_line=lines.append,
    )


class DesktopBridgeDoctorReportTests(unittest.TestCase):
    def test_print_doctor_report_renders_expected_window_lookup_failure(self) -> None:
        lines: list[str] = []

        def find_window() -> WindowInfo:
            raise WindowMissingError("window missing")

        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            doctor.print_doctor_report(2, _config(Path(temp_dir)), _deps(lines, find_window))

        self.assertIn("codex_window_found: False", lines)
        self.assertIn("codex_window_error: window missing", lines)
        self.assertIn("codex_app_package_version: 26.612.1.0", lines)
        self.assertIn("codex_app_previous_package_version: 26.611.8604.0", lines)
        self.assertIn("codex_app_update_detected: True", lines)
        self.assertIn("codex_app_restart_recommended: True", lines)

    def test_print_doctor_report_surfaces_unexpected_window_probe_failure(self) -> None:
        lines: list[str] = []

        def find_window() -> WindowInfo:
            raise UnexpectedWindowProbeError("probe failed")

        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            with self.assertRaisesRegex(UnexpectedWindowProbeError, "probe failed"):
                doctor.print_doctor_report(2, _config(Path(temp_dir)), _deps(lines, find_window))

        self.assertNotIn("codex_window_found: False", lines)


if __name__ == "__main__":
    _ = unittest.main()
