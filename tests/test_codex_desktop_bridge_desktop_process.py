# pyright: reportAny=false, reportAttributeAccessIssue=false, reportPrivateUsage=false, reportUnknownArgumentType=false, reportUnknownLambdaType=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnusedCallResult=false
from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path
from typing import cast
from unittest.mock import patch

import codex_desktop_bridge as bridge
import codex_desktop_bridge_desktop_process as desktop_process
import codex_desktop_bridge_desktop_process_scripts as desktop_process_scripts
import codex_desktop_bridge_desktop_resolver as desktop_resolver


class DesktopProcessTests(unittest.TestCase):
    def test_discovery_precedence_and_ensure_persists_env(self) -> None:
        env_path = Path("C:/Codex/env/Codex.exe")
        cli_path = Path("C:/Codex/app/resources/codex.exe")
        running_path = Path("C:/Codex/running/Codex.exe")
        powershell_path = Path("C:/Codex/pwsh/Codex.exe")
        registry_path = Path("C:/Codex/registry/Codex.exe")
        default_path = Path("C:/Codex/default/Codex.exe")

        self.assertEqual(
            desktop_process.discover_codex_desktop_executable(
                env_name="CODEX_DESKTOP_EXE",
                deps=_deps(
                    env_path=env_path,
                    running=(running_path, "running"),
                    powershell=(powershell_path, "powershell"),
                    registry=[("registry", registry_path)],
                    defaults=[("default", default_path)],
                ),
            ),
            (env_path, "env:CODEX_DESKTOP_EXE"),
        )
        self.assertEqual(
            desktop_process.discover_codex_desktop_executable(
                env_name="CODEX_DESKTOP_EXE",
                deps=_deps(
                    env_path=cli_path,
                    running=(cli_path, "running-cli"),
                    powershell=(powershell_path, "powershell"),
                ),
            ),
            (powershell_path, "powershell"),
        )
        self.assertFalse(desktop_process.is_codex_desktop_executable_candidate(cli_path))
        self.assertTrue(desktop_process.is_codex_desktop_executable_candidate(env_path))
        self.assertEqual(
            desktop_process.discover_codex_desktop_executable(
                env_name="CODEX_DESKTOP_EXE",
                deps=_deps(powershell=(powershell_path, "powershell"), registry=[("registry", registry_path)]),
            ),
            (powershell_path, "powershell"),
        )

        persisted: list[tuple[Path, str, str]] = []
        environ: dict[str, str] = {}
        ensured = desktop_process.ensure_codex_desktop_executable_configured(
            bridge_env_path=Path(".env"),
            env_name="CODEX_DESKTOP_EXE",
            deps=_deps(
                registry=[("registry", registry_path)],
                persist=lambda env_file, name, value: persisted.append((env_file, name, value)) or True,
                set_env=lambda name, value: environ.__setitem__(name, value),
            ),
        )
        self.assertEqual(ensured, (registry_path, "registry", True))
        self.assertEqual(persisted, [(Path(".env"), "CODEX_DESKTOP_EXE", str(registry_path))])
        self.assertEqual(environ["CODEX_DESKTOP_EXE"], str(registry_path))

    def test_process_stop_start_and_bridge_discovery_wrapper(self) -> None:
        run_calls: list[list[str]] = []
        start_calls: list[dict[str, object]] = []
        started = cast(subprocess.Popen[str], object())
        deps = _deps(
            which=lambda name: f"C:/Windows/{name}.exe",
            run=lambda args, **_kwargs: run_calls.append(args)
            or subprocess.CompletedProcess(args=args, returncode=0, stdout="stopped", stderr=""),
            start=lambda args, **kwargs: start_calls.append({"args": args, **kwargs}) or started,
        )

        with patch.object(desktop_process.sys, "platform", "win32"):
            stopped, details = desktop_process.stop_codex_desktop_processes(Path("C:/Codex/Codex.exe"), deps=deps)
            self.assertTrue(stopped)
            self.assertEqual(details, "stopped")
            self.assertEqual(run_calls, [["C:/Windows/taskkill.exe", "/IM", "Codex.exe", "/F"]])

            self.assertIs(desktop_process.start_codex_desktop_process(Path("C:/Codex/Codex.exe"), deps=deps), started)
        self.assertEqual(start_calls[0]["args"], [str(Path("C:/Codex/Codex.exe"))])
        self.assertEqual(start_calls[0]["cwd"], str(Path("C:/Codex")))
        self.assertTrue(start_calls[0]["close_fds"])

        original_env = bridge.get_optional_env_file_path
        try:
            bridge.get_optional_env_file_path = lambda _name: Path("C:/Codex/Codex.exe")
            self.assertEqual(
                bridge.discover_codex_desktop_executable(),
                (Path("C:/Codex/Codex.exe"), "env:CODEX_DESKTOP_EXE"),
            )
        finally:
            bridge.get_optional_env_file_path = original_env

    def test_no_discovery_ensure_failure_and_stop_details_edges(self) -> None:
        self.assertEqual(
            desktop_process.discover_codex_desktop_executable(env_name="CODEX_DESKTOP_EXE", deps=_deps()),
            (None, ""),
        )
        with self.assertRaisesRegex(RuntimeError, "Codex Desktop executable could not be discovered"):
            _ = desktop_process.ensure_codex_desktop_executable_configured(
                bridge_env_path=Path(".env"),
                env_name="CODEX_DESKTOP_EXE",
                deps=_deps(),
            )

        empty_stop = desktop_process.stop_codex_desktop_processes(
            Path("C:/Codex/Codex.exe"),
            deps=_deps(run=lambda args, **_kwargs: subprocess.CompletedProcess(args=args, returncode=1, stdout="", stderr="")),
        )
        self.assertEqual(empty_stop, (False, "-"))

        stderr_stop = desktop_process.stop_codex_desktop_processes(
            Path("C:/Codex/Codex.exe"),
            deps=_deps(
                run=lambda args, **_kwargs: subprocess.CompletedProcess(args=args, returncode=1, stdout="", stderr="denied")
            ),
        )
        self.assertEqual(stderr_stop, (False, "denied"))

    def test_app_server_stop_script_builder_contains_process_filter(self) -> None:
        script = desktop_process_scripts.build_stop_codex_app_server_script()

        self.assertIn("Get-CimInstance Win32_Process", script)
        self.assertIn("$_.Name -ieq 'codex.exe' -and $_.CommandLine -match 'app-server'", script)
        self.assertIn("Write-Output 'no_matching_app_servers'", script)
        self.assertIn("Stop-Process -Id $process.ProcessId -Force -ErrorAction Stop", script)
        self.assertIn('Write-Output ("stopped PID={0} EXE={1}"', script)
        self.assertIn('Write-Output ("stop_failed PID={0}: {1}"', script)

    def test_app_server_stop_preserves_skip_and_powershell_result_behavior(self) -> None:
        run_args: list[list[str]] = []
        run_kwargs: list[dict[str, str | bool | int]] = []
        with (
            patch("codex_desktop_bridge_desktop_process.os.name", "posix"),
            patch.object(desktop_process.sys, "platform", "linux"),
        ):
            self.assertEqual(
                desktop_process.stop_codex_app_server_processes(),
                (False, "skipped: app-server process stop is only implemented on Windows"),
            )

        def fake_run(args: list[str], **kwargs: str | bool | int) -> subprocess.CompletedProcess[str]:
            run_args.append(args)
            run_kwargs.append(dict(kwargs))
            return subprocess.CompletedProcess(
                args=args,
                returncode=0,
                stdout="no_matching_app_servers\nstopped PID=123 EXE=C:/Codex/codex.exe\n",
                stderr="warning\n",
            )

        with (
            patch("codex_desktop_bridge_desktop_process.os.name", "nt"),
            patch.object(desktop_process.sys, "platform", "win32"),
            patch("codex_desktop_bridge_desktop_process.subprocess.run", fake_run),
        ):
            stopped, details = desktop_process.stop_codex_app_server_processes()

        self.assertTrue(stopped)
        self.assertEqual(details, "no_matching_app_servers\nstopped PID=123 EXE=C:/Codex/codex.exe\nwarning")
        self.assertEqual(run_args[0][:3], ["powershell", "-NoProfile", "-Command"])
        self.assertEqual(run_args[0][3], desktop_process_scripts.build_stop_codex_app_server_script())
        self.assertTrue(run_kwargs[0]["capture_output"])
        self.assertTrue(run_kwargs[0]["text"])
        self.assertEqual(run_kwargs[0]["encoding"], "utf-8")
        self.assertEqual(run_kwargs[0]["errors"], "replace")
        self.assertEqual(run_kwargs[0]["creationflags"], getattr(subprocess, "CREATE_NO_WINDOW", 0))

    def test_default_candidates_deduplicate_repeated_env_roots(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            base = Path(temp_dir)
            first = base / "Programs" / "Codex" / "Codex.exe"
            second = base / "Codex" / "Codex.exe"
            first.parent.mkdir(parents=True)
            second.parent.mkdir(parents=True)
            first.write_text("", encoding="utf-8")
            second.write_text("", encoding="utf-8")

            candidates = list(
                desktop_process.iter_default_codex_desktop_candidates(
                    {"LOCALAPPDATA": temp_dir, "ProgramFiles": temp_dir, "ProgramFiles(x86)": temp_dir}
                )
            )

        self.assertEqual(candidates, [(f"default:{first.parent}", first), (f"default:{second.parent}", second)])

    def test_macos_app_bundle_resolution_and_start_stop_commands(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            app_bundle = Path(temp_dir) / "Codex.app"
            executable = app_bundle / "Contents" / "MacOS" / "Codex"
            executable.parent.mkdir(parents=True)
            executable.write_text("", encoding="utf-8")

            with patch.object(desktop_resolver.sys, "platform", "darwin"):
                self.assertEqual(desktop_resolver.normalize_executable_candidate(str(app_bundle)), executable)

            run_calls: list[list[str]] = []
            start_calls: list[dict[str, object]] = []
            started = cast(subprocess.Popen[str], object())
            deps = _deps(
                run=lambda args, **_kwargs: run_calls.append(args)
                or subprocess.CompletedProcess(args=args, returncode=0, stdout="", stderr=""),
                start=lambda args, **kwargs: start_calls.append({"args": args, **kwargs}) or started,
            )

            with patch.object(desktop_process.sys, "platform", "darwin"):
                self.assertEqual(desktop_process.stop_codex_desktop_processes(executable, deps=deps), (True, "-"))
                self.assertIs(desktop_process.start_codex_desktop_process(executable, deps=deps), started)

            self.assertEqual(run_calls[0], ["osascript", "-e", 'tell application "Codex" to quit'])
            self.assertEqual(start_calls[0]["args"], ["open", str(app_bundle)])


def _deps(
    *,
    env_path: Path | None = None,
    running: tuple[Path | None, str] = (None, ""),
    powershell: tuple[Path | None, str] = (None, ""),
    registry: list[tuple[str, Path]] | None = None,
    defaults: list[tuple[str, Path]] | None = None,
    persist: desktop_process.PersistEnvValue | None = None,
    set_env: desktop_process.SetEnvironValue | None = None,
    which: desktop_process.WhichExecutable | None = None,
    run: desktop_process.RunProcess | None = None,
    start: desktop_process.StartProcess | None = None,
) -> desktop_process.DesktopProcessDeps:
    started = cast(subprocess.Popen[str], object())
    return desktop_process.DesktopProcessDeps(
        get_optional_env_file_path=lambda _name: env_path,
        detect_running_codex_desktop_executable=lambda: running,
        detect_codex_desktop_executable_via_powershell=lambda: powershell,
        iter_codex_desktop_registry_candidates=lambda: registry or [],
        iter_default_codex_desktop_candidates=lambda: defaults or [],
        persist_env_value=persist or (lambda _env_file, _name, _value: False),
        set_environ_value=set_env or (lambda _name, _value: None),
        which=which or (lambda _name: None),
        run_process=run
        or (lambda args, **_kwargs: subprocess.CompletedProcess(args=args, returncode=0, stdout="", stderr="")),
        start_process=start or (lambda args, **_kwargs: started),
    )


if __name__ == "__main__":
    unittest.main()
