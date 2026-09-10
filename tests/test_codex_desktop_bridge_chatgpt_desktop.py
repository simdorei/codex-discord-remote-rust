from __future__ import annotations

import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path
from unittest.mock import patch

import codex_desktop_bridge_desktop_process as desktop_process
import codex_desktop_bridge_desktop_resolver as desktop_resolver


@dataclass(frozen=True, slots=True)
class FakeLauncher:
    pid: int = 4321

    def poll(self) -> int:
        return 1


def test_powershell_discovery_prefers_chatgpt_process() -> None:
    with tempfile.TemporaryDirectory() as temp_dir:
        executable = Path(temp_dir) / "ChatGPT.exe"
        _ = executable.write_text("", encoding="utf-8")
        commands: list[str] = []

        def capture(command: str) -> str:
            commands.append(command)
            if "Get-Process" in command and "ChatGPT" in command:
                return str(executable)
            return ""

        with (
            patch("codex_desktop_bridge_desktop_resolver.os.name", "nt"),
            patch("codex_desktop_bridge_desktop_resolver.sys.platform", "win32"),
            patch.object(desktop_resolver, "run_powershell_capture", capture),
        ):
            discovered = (
                desktop_resolver.detect_codex_desktop_executable_via_powershell()
            )

    assert discovered == (executable, "powershell:Get-Process")
    assert "ChatGPT" in commands[0]
    assert commands[0].index("ChatGPT") < commands[0].index("Codex")


def test_appx_discovery_prefers_chatgpt_executable_with_legacy_fallback() -> None:
    with tempfile.TemporaryDirectory() as temp_dir:
        install_root = Path(temp_dir)
        chatgpt_executable = install_root / "app" / "ChatGPT.exe"
        codex_executable = install_root / "app" / "Codex.exe"
        chatgpt_executable.parent.mkdir(parents=True)
        _ = chatgpt_executable.write_text("", encoding="utf-8")
        _ = codex_executable.write_text("", encoding="utf-8")

        def capture(command: str) -> str:
            return str(install_root) if "Get-AppxPackage" in command else ""

        with (
            patch("codex_desktop_bridge_desktop_resolver.os.name", "nt"),
            patch("codex_desktop_bridge_desktop_resolver.sys.platform", "win32"),
            patch.object(desktop_resolver, "run_powershell_capture", capture),
        ):
            discovered = (
                desktop_resolver.detect_codex_desktop_executable_via_powershell()
            )

    assert discovered == (chatgpt_executable, "powershell:Get-AppxPackage")


def test_packaged_chatgpt_launch_uses_registered_apps_folder_identity() -> None:
    with tempfile.TemporaryDirectory() as temp_dir:
        package_root = (
            Path(temp_dir)
            / "WindowsApps"
            / "OpenAI.Codex_26.715.7063.0_x64__2p2nqsd0c76g0"
        )
        executable = package_root / "app" / "ChatGPT.exe"
        executable.parent.mkdir(parents=True)
        _ = executable.write_text("", encoding="utf-8")
        calls: list[list[str]] = []
        launcher = FakeLauncher()

        def start(
            args: list[str], **_kwargs: str | bool | int
        ) -> desktop_process.StartedDesktopProcess:
            calls.append(args)
            return launcher

        def run(
            args: list[str], **_kwargs: str | bool | int
        ) -> subprocess.CompletedProcess[str]:
            return subprocess.CompletedProcess(
                args=args, returncode=0, stdout="", stderr=""
            )

        deps = desktop_process.DesktopProcessDeps(
            get_optional_env_file_path=lambda _name: None,
            detect_running_codex_desktop_executable=lambda: (None, ""),
            detect_codex_desktop_executable_via_powershell=lambda: (None, ""),
            iter_codex_desktop_registry_candidates=lambda: [],
            iter_default_codex_desktop_candidates=lambda: [],
            persist_env_value=lambda _path, _name, _value: False,
            set_environ_value=lambda _name, _value: None,
            which=lambda name: f"C:/Windows/{name}.exe",
            run_process=run,
            start_process=start,
        )

        with patch("codex_desktop_bridge_desktop_process.sys.platform", "win32"):
            started = desktop_process.start_codex_desktop_process(executable, deps=deps)

    assert calls == [
        [
            "C:/Windows/explorer.exe",
            "shell:AppsFolder\\OpenAI.Codex_2p2nqsd0c76g0!App",
        ]
    ]
    assert started.pid == launcher.pid
    assert started.poll() is None
