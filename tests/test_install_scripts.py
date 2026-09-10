from __future__ import annotations

import os
import shutil
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class InstallScriptTests(unittest.TestCase):
    def test_windows_install_bootstraps_python_312(self) -> None:
        text = (ROOT / "install.ps1").read_text(encoding="utf-8")

        self.assertIn("$RequiredPythonMajor = 3", text)
        self.assertIn("$RequiredPythonMinor = 12", text)
        self.assertIn("$PortablePythonVersion = '3.12.1'", text)
        self.assertIn(".python-portable", text)
        self.assertIn("python-${PortablePythonVersion}-embed-amd64.zip", text)
        manifest = (ROOT / "runtime-release.json").read_text(encoding="utf-8")
        self.assertIn("python.org/ftp/python/3.12.1", manifest)
        self.assertIn("bootstrap.pypa.io/get-pip.py", manifest)
        self.assertIn("get-pip.py", text)
        self.assertIn("get-pip.log", text)
        self.assertIn("runtime-release.json", text)
        self.assertIn("function Assert-FileSha256", text)
        self.assertIn("Get-FileHash", text)
        self.assertIn(".python-portable.stage", text)
        self.assertIn("Move-Item", text)
        self.assertIn("pip==$PortablePipVersion", text)
        self.assertIn("--require-hashes", text)
        self.assertIn("$updatedLines.Add('..')", text)
        self.assertIn("Set-EnvFileValue -Name 'PYTHON_EXE'", text)
        self.assertIn("Set-EnvFileValue -Name 'CODEX_HOME'", text)
        self.assertIn("Set-EnvFileValue -Name 'CODEX_EXE'", text)
        self.assertIn("$CodexExeWasExplicit", text)
        self.assertNotIn(
            "Set-EnvFileValue -Name 'CODEX_EXE' -Value $codexCommand", text
        )
        self.assertIn("PATH-discovered Codex command was not saved", text)
        self.assertIn("Would set CODEX_HOME=$codexHomePath in .env", text)
        self.assertIn("function Test-CodexRuntimeBinPath", text)
        self.assertIn("\\.sandbox-bin", text)
        self.assertIn("\\plugins\\.plugin-appserver", text)
        self.assertIn("\\appdata\\local\\openai\\codex\\bin", text)
        self.assertNotIn("winget", text)
        self.assertNotIn("Install Python 3.11+", text)

    def test_windows_launcher_uses_python_312_only(self) -> None:
        text = (ROOT / "codex-discord-bot.cmd").read_text(encoding="utf-8")

        self.assertIn(r".python-portable\python.exe", text)
        self.assertIn("CODEX_HOME", text)
        self.assertIn("CODEX_EXE", text)
        self.assertIn("CODEX_DESKTOP_EXE", text)
        self.assertIn("Portable Python 3.12 was not found", text)
        self.assertNotIn("Python313", text)
        self.assertNotIn('py -3 "%SCRIPT%"', text)
        self.assertNotIn("py -3.12", text)
        self.assertNotIn('python "%SCRIPT%"', text)

    def test_setup_wrappers_require_python_312(self) -> None:
        powershell = (ROOT / "setup-discord-bot.ps1").read_text(encoding="utf-8")
        shell = (ROOT / "setup-discord-bot.sh").read_text(encoding="utf-8")

        self.assertIn(r".python-portable\python.exe", powershell)
        self.assertIn("Portable Python 3.12 was not found", powershell)
        self.assertIn("Register-ScheduledTask", powershell)
        self.assertIn("'Codex Discord Bot'", powershell)
        self.assertIn("codex-discord-watchdog.ps1", powershell)
        self.assertIn("RepetitionInterval", powershell)
        self.assertIn("MultipleInstances IgnoreNew", powershell)
        self.assertIn("python3.12", shell)
        self.assertIn("Python 3.12 was not found", shell)

    def test_macos_install_requires_python_312_without_auto_install(self) -> None:
        text = (ROOT / "install.sh").read_text(encoding="utf-8")

        self.assertIn("required_python_minor=12", text)
        self.assertIn("Would require Python 3.12 on PATH or --python-exe", text)
        self.assertIn("set_env_value PYTHON_EXE", text)
        self.assertIn("set_env_value CODEX_HOME", text)
        self.assertIn("set_env_value CODEX_EXE", text)
        self.assertIn("codex_exe_was_explicit=1", text)
        self.assertNotIn('set_env_value CODEX_EXE "$codex_command"', text)
        self.assertIn("PATH-discovered Codex command was not saved", text)
        self.assertIn("default_codex_home=\"$HOME/.codex\"", text)
        self.assertIn("/.sandbox-bin", text)
        self.assertIn("/plugins/.plugin-appserver", text)
        self.assertIn("/appdata/local/openai/codex/bin", text)
        self.assertIn("--require-hashes", text)
        self.assertNotIn("brew install", text)
        self.assertNotIn("Install Python 3.11+", text)

    @unittest.skipUnless(os.name == "nt", "Windows cmd launcher test")
    def test_windows_launcher_exports_codex_paths_from_env(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            launcher = temp_path / "codex-discord-bot.cmd"
            _ = launcher.write_text(
                (ROOT / "codex-discord-bot.cmd").read_text(encoding="utf-8"),
                encoding="utf-8",
            )
            _ = (temp_path / "codex_discord_bot.py").write_text("", encoding="utf-8")
            fake_python = temp_path / "fake python.cmd"
            capture = temp_path / "capture.txt"
            _ = fake_python.write_text(
                "\n".join(
                    (
                        "@echo off",
                        "> \"%HARNESS_CAPTURE%\" echo CODEX_HOME=%CODEX_HOME%",
                        ">> \"%HARNESS_CAPTURE%\" echo CODEX_EXE=%CODEX_EXE%",
                        ">> \"%HARNESS_CAPTURE%\" echo CODEX_DESKTOP_EXE=%CODEX_DESKTOP_EXE%",
                        ">> \"%HARNESS_CAPTURE%\" echo PYTHON_EXE=%PYTHON_EXE%",
                        "exit /b 0",
                    )
                )
                + "\n",
                encoding="utf-8",
            )
            _ = (temp_path / ".env").write_text(
                "\n".join(
                    (
                        f"PYTHON_EXE={fake_python}",
                        f"CODEX_HOME={temp_path / 'Codex Home'}",
                        f"CODEX_EXE={temp_path / 'Codex Bin' / 'codex.exe'}",
                        f"CODEX_DESKTOP_EXE={temp_path / 'Codex Desktop' / 'Codex.exe'}",
                    )
                )
                + "\n",
                encoding="utf-8",
            )
            env = os.environ.copy()
            for name in ("PYTHON_EXE", "CODEX_HOME", "CODEX_EXE", "CODEX_DESKTOP_EXE"):
                env.pop(name, None)
            env["CODEX_DISCORD_RUNTIME"] = "python"
            env["HARNESS_CAPTURE"] = str(capture)

            completed = subprocess.run(
                ["cmd.exe", "/c", str(launcher)],
                cwd=temp_path,
                env=env,
                capture_output=True,
                text=True,
                errors="replace",
                timeout=30.0,
                check=False,
            )

            self.assertEqual(completed.returncode, 0, completed.stderr + completed.stdout)
            captured = capture.read_text(encoding="utf-8")
            self.assertIn(f"CODEX_HOME={temp_path / 'Codex Home'}", captured)
            self.assertIn(f"CODEX_EXE={temp_path / 'Codex Bin' / 'codex.exe'}", captured)
            self.assertIn(f"CODEX_DESKTOP_EXE={temp_path / 'Codex Desktop' / 'Codex.exe'}", captured)
            self.assertIn(f"PYTHON_EXE={fake_python}", captured)

    @unittest.skipIf(shutil.which("sh") is None, "sh launcher test requires sh")
    def test_shell_launcher_exports_codex_paths_from_env(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            launcher = temp_path / "codex-discord-bot.sh"
            _ = launcher.write_text(
                (ROOT / "codex-discord-bot.sh").read_text(encoding="utf-8"),
                encoding="utf-8",
            )
            launcher.chmod(launcher.stat().st_mode | stat.S_IXUSR)
            _ = (temp_path / "codex_discord_bot.py").write_text("", encoding="utf-8")
            fake_python = temp_path / "fake python"
            capture = temp_path / "capture.txt"
            _ = fake_python.write_text(
                "\n".join(
                    (
                        "#!/usr/bin/env sh",
                        "if [ \"$1\" = \"-c\" ]; then exit 0; fi",
                        "{",
                        "  printf 'CODEX_HOME=%s\\n' \"$CODEX_HOME\"",
                        "  printf 'CODEX_EXE=%s\\n' \"$CODEX_EXE\"",
                        "  printf 'CODEX_DESKTOP_EXE=%s\\n' \"$CODEX_DESKTOP_EXE\"",
                        "  printf 'PYTHON_EXE=%s\\n' \"$PYTHON_EXE\"",
                        "} > \"$HARNESS_CAPTURE\"",
                        "exit 0",
                    )
                )
                + "\n",
                encoding="utf-8",
            )
            fake_python.chmod(fake_python.stat().st_mode | stat.S_IXUSR)
            _ = (temp_path / ".env").write_text(
                "\n".join(
                    (
                        f"PYTHON_EXE={fake_python}",
                        f"CODEX_HOME={temp_path / 'Codex Home'}",
                        f"CODEX_EXE={temp_path / 'Codex Bin' / 'codex'}",
                        f"CODEX_DESKTOP_EXE={temp_path / 'Codex Desktop' / 'Codex.app'}",
                    )
                )
                + "\n",
                encoding="utf-8",
            )
            env = os.environ.copy()
            env["HARNESS_CAPTURE"] = str(capture)

            completed = subprocess.run(
                ["sh", str(launcher)],
                cwd=temp_path,
                env=env,
                capture_output=True,
                text=True,
                timeout=30.0,
                check=False,
            )

            self.assertEqual(completed.returncode, 0, completed.stderr + completed.stdout)
            captured = capture.read_text(encoding="utf-8")
            self.assertIn(f"CODEX_HOME={temp_path / 'Codex Home'}", captured)
            self.assertIn(f"CODEX_EXE={temp_path / 'Codex Bin' / 'codex'}", captured)
            self.assertIn(f"CODEX_DESKTOP_EXE={temp_path / 'Codex Desktop' / 'Codex.app'}", captured)
            self.assertIn(f"PYTHON_EXE={fake_python}", captured)


if __name__ == "__main__":
    _ = unittest.main()
