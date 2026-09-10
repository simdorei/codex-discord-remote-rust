# pyright: reportAny=false, reportAttributeAccessIssue=false, reportPrivateUsage=false, reportUnknownArgumentType=false, reportUnknownLambdaType=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnusedCallResult=false
from __future__ import annotations

import subprocess
import unittest
from dataclasses import dataclass
from pathlib import Path
from typing import TypedDict, Unpack

import codex_desktop_bridge as bridge
import codex_desktop_bridge_sidebar_activation as sidebar_activation
from codex_thread_models import WindowInfo


class UnexpectedRunProcessError(Exception):
    pass


class RunProcessKwargs(TypedDict):
    capture_output: bool
    text: bool
    encoding: str
    errors: str
    creationflags: int
    timeout: float
    check: bool
    env: dict[str, str]


CallValue = str | int | float
Call = tuple[CallValue, ...]


class SidebarActivationHappyTests(unittest.TestCase):
    def test_legacy_and_v2_run_resource_scripts_and_bridge_wrappers(self) -> None:
        legacy = _harness(stdout="OK:Legacy row")
        self.assertEqual(
            sidebar_activation.legacy_activate_thread_by_sidebar("Thread", "repo", legacy.deps),
            "Legacy row",
        )
        self.assertEqual(legacy.calls, [("read", "legacy.ps1"), ("run", "legacy.ps1", 15.0)])
        self.assertEqual(legacy.environ["CODEX_THREAD_NAME"], "Thread")
        self.assertEqual(legacy.environ["CODEX_PROJECT_NAME"], "repo")

        v2 = _harness(stdout="OK:V2 row")
        self.assertEqual(
            sidebar_activation.activate_thread_by_sidebar_v2("Thread", "repo", v2.deps),
            "V2 row",
        )
        self.assertEqual(
            v2.calls,
            [("focus", 7), ("read", "v2.ps1"), ("run", "v2.ps1", 25.0)],
        )
        self.assertEqual(v2.environ["CODEX_WINDOW_HANDLE"], "7")

        original_deps = bridge._make_sidebar_activation_deps
        try:
            bridge._make_sidebar_activation_deps = lambda: _harness(stdout="OK:Wrapped row").deps
            self.assertEqual(bridge.activate_thread_by_sidebar_v2("Thread", "repo"), "Wrapped row")
        finally:
            bridge._make_sidebar_activation_deps = original_deps


class SidebarActivationEdgeTests(unittest.TestCase):
    def test_activation_failures_surface_existing_details(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "Missing thread_name"):
            _ = sidebar_activation.legacy_activate_thread_by_sidebar("   ", None, _harness().deps)

        with self.assertRaisesRegex(RuntimeError, "Sidebar activation failed: NOT_FOUND:Thread"):
            _ = sidebar_activation.legacy_activate_thread_by_sidebar(
                "Thread",
                None,
                _harness(returncode=6, stdout="NOT_FOUND:Thread").deps,
            )

        with self.assertRaisesRegex(RuntimeError, "Sidebar activation failed: denied"):
            _ = sidebar_activation.legacy_activate_thread_by_sidebar(
                "Thread",
                None,
                _harness(returncode=4, stdout="", stderr="denied").deps,
            )

        with self.assertRaisesRegex(RuntimeError, "Sidebar activation failed: exit=9"):
            _ = sidebar_activation.legacy_activate_thread_by_sidebar(
                "Thread",
                None,
                _harness(returncode=9, stdout="", stderr="").deps,
            )

        def failing_run(args: list[str], **kwargs: Unpack[RunProcessKwargs]) -> subprocess.CompletedProcess[str]:
            _ = (args, kwargs)
            raise subprocess.TimeoutExpired(cmd=args, timeout=15.0)

        with self.assertRaisesRegex(RuntimeError, "Sidebar activation subprocess failed:"):
            _ = sidebar_activation.legacy_activate_thread_by_sidebar(
                "Thread",
                None,
                _harness(run=failing_run).deps,
            )

        def unexpected_run(args: list[str], **kwargs: Unpack[RunProcessKwargs]) -> subprocess.CompletedProcess[str]:
            _ = (args, kwargs)
            raise UnexpectedRunProcessError("run dependency broke")

        with self.assertRaisesRegex(UnexpectedRunProcessError, "run dependency broke"):
            _ = sidebar_activation.legacy_activate_thread_by_sidebar(
                "Thread",
                None,
                _harness(run=unexpected_run).deps,
            )

        def missing_resource(_path: Path) -> str:
            raise OSError("missing resource")

        with self.assertRaisesRegex(OSError, "missing resource"):
            _ = sidebar_activation.legacy_activate_thread_by_sidebar(
                "Thread",
                None,
                _harness(read_text=missing_resource).deps,
            )


@dataclass(frozen=True, slots=True)
class SidebarHarness:
    deps: sidebar_activation.SidebarActivationDeps
    calls: list[Call]
    environ: dict[str, str]


def _harness(
    *,
    returncode: int = 0,
    stdout: str = "OK:Thread row",
    stderr: str = "",
    run: sidebar_activation.RunProcess | None = None,
    read_text: sidebar_activation.ReadText | None = None,
) -> SidebarHarness:
    calls: list[Call] = []
    environ: dict[str, str] = {}
    legacy_path = Path("legacy.ps1")
    v2_path = Path("v2.ps1")

    def default_read_text(path: Path) -> str:
        calls.append(("read", path.name))
        return "Write-Output 'OK:Thread row'"

    def run_process(args: list[str], **kwargs: Unpack[RunProcessKwargs]) -> subprocess.CompletedProcess[str]:
        path = "legacy.ps1" if args[-1] == "Write-Output 'OK:Thread row'" else "unknown"
        if kwargs["timeout"] == 25.0:
            path = "v2.ps1"
        calls.append(("run", path, kwargs["timeout"]))
        env = kwargs["env"]
        environ.update(env)
        return subprocess.CompletedProcess(args=args, returncode=returncode, stdout=stdout, stderr=stderr)

    deps = sidebar_activation.SidebarActivationDeps(
        legacy_script_path=legacy_path,
        v2_script_path=v2_path,
        read_text=read_text or default_read_text,
        find_codex_window=lambda: WindowInfo(hwnd=7, title="Codex", left=0, top=0, right=100, bottom=100),
        focus_window=lambda window: calls.append(("focus", window.hwnd)),
        run_process=run or run_process,
        environ_copy=lambda: {},
        create_no_window=0,
    )
    return SidebarHarness(deps=deps, calls=calls, environ=environ)


if __name__ == "__main__":
    unittest.main()
