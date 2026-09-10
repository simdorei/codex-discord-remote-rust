from __future__ import annotations

import os
import shutil
import stat
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def _copy_installer_fixture(temp_path: Path, installer_name: str) -> Path:
    installer = temp_path / installer_name
    _ = installer.write_text(
        (ROOT / installer_name).read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    for name in ("runtime-release.json", ".env.example"):
        _ = (temp_path / name).write_text(
            (ROOT / name).read_text(encoding="utf-8"),
            encoding="utf-8",
        )
    return installer


def _env_value(path: Path, name: str) -> str | None:
    for line in path.read_text(encoding="utf-8-sig").splitlines():
        key, separator, value = line.partition("=")
        if separator and key.strip() == name:
            return value
    return None


@unittest.skipUnless(os.name == "nt", "PowerShell installer test runs on Windows")
@unittest.skipUnless(shutil.which("powershell.exe"), "powershell.exe is required")
class WindowsCodexExePersistenceTests(unittest.TestCase):
    def _run_installer(
        self,
        temp_path: Path,
        *,
        explicit_codex: Path | None = None,
        inherited_codex: Path | None = None,
    ) -> subprocess.CompletedProcess[str]:
        installer = _copy_installer_fixture(temp_path, "install.ps1")
        fake_python = temp_path / "fake-python.cmd"
        _ = fake_python.write_text(
            textwrap.dedent(
                """\
                @echo off
                if "%~1"=="-c" echo %~f0
                exit /b 0
                """
            ).replace("\n", "\r\n"),
            encoding="utf-8",
        )
        path_codex = temp_path / "codex.cmd"
        _ = path_codex.write_text("@exit /b 0\r\n", encoding="utf-8")
        arguments = [
            shutil.which("powershell.exe") or "powershell.exe",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            str(installer),
            "-Runtime",
            "python",
            "-PythonExe",
            str(fake_python),
            "-SkipDependencies",
            "-SkipCodexPlugin",
        ]
        if explicit_codex is not None:
            arguments.extend(("-CodexExe", str(explicit_codex)))
        env = os.environ.copy()
        env.pop("CODEX_EXE", None)
        if inherited_codex is not None:
            env["CODEX_EXE"] = str(inherited_codex)
        env["PATH"] = str(temp_path) + os.pathsep + env.get("PATH", "")
        return subprocess.run(
            arguments,
            cwd=temp_path,
            env=env,
            capture_output=True,
            encoding="utf-8",
            errors="replace",
            timeout=30,
            check=False,
        )

    def test_path_discovery_does_not_pin_codex_exe(self) -> None:
        """CEX-1/CEX-3: PATH discovery is transient and other env data survives."""
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            env_path = temp_path / ".env"
            _ = env_path.write_text(
                "SENTINEL=keep\nCODEX_EXE=\n",
                encoding="utf-8",
            )
            completed = self._run_installer(temp_path)
            codex_exe = _env_value(env_path, "CODEX_EXE")
            sentinel = _env_value(env_path, "SENTINEL")

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertEqual(codex_exe, "")
        self.assertEqual(sentinel, "keep")
        self.assertIn("PATH-discovered Codex command was not saved", output)

    def test_explicit_codex_exe_is_persisted(self) -> None:
        """CEX-2: an explicit installer override remains supported."""
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            explicit_codex = temp_path / "explicit-codex.cmd"
            _ = explicit_codex.write_text("@exit /b 0\r\n", encoding="utf-8")
            completed = self._run_installer(
                temp_path,
                explicit_codex=explicit_codex,
            )
            codex_exe = _env_value(temp_path / ".env", "CODEX_EXE")

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertEqual(codex_exe, str(explicit_codex))

    def test_inherited_codex_exe_does_not_become_a_persistent_pin(self) -> None:
        """CEX-1: Codex-injected process state is not mistaken for user intent."""
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            inherited_codex = temp_path / "inherited-codex.cmd"
            _ = inherited_codex.write_text("@exit /b 0\r\n", encoding="utf-8")
            completed = self._run_installer(
                temp_path,
                inherited_codex=inherited_codex,
            )
            codex_exe = _env_value(temp_path / ".env", "CODEX_EXE")

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertEqual(codex_exe, "")
        self.assertIn("Inherited CODEX_EXE was not saved", output)


@unittest.skipIf(os.name == "nt", "shell installer requires POSIX path semantics")
@unittest.skipIf(shutil.which("sh") is None, "sh is required")
class ShellCodexExePersistenceTests(unittest.TestCase):
    def _run_installer(
        self,
        temp_path: Path,
        *,
        explicit_codex: Path | None = None,
        inherited_codex: Path | None = None,
    ) -> subprocess.CompletedProcess[str]:
        installer = _copy_installer_fixture(temp_path, "install.sh")
        installer.chmod(installer.stat().st_mode | stat.S_IXUSR)
        fake_python = temp_path / "fake-python"
        _ = fake_python.write_text(
            textwrap.dedent(
                """\
                #!/usr/bin/env sh
                if [ "${1:-}" = "-c" ]; then printf '%s\n' "$0"; fi
                exit 0
                """
            ),
            encoding="utf-8",
        )
        fake_python.chmod(fake_python.stat().st_mode | stat.S_IXUSR)
        path_codex = temp_path / "codex"
        _ = path_codex.write_text("#!/usr/bin/env sh\nexit 0\n", encoding="utf-8")
        path_codex.chmod(path_codex.stat().st_mode | stat.S_IXUSR)
        arguments = [
            "sh",
            str(installer),
            "--python-exe",
            str(fake_python),
            "--skip-dependencies",
            "--skip-codex-plugin",
        ]
        if explicit_codex is not None:
            arguments.extend(("--codex-exe", str(explicit_codex)))
        env = os.environ.copy()
        env.pop("CODEX_EXE", None)
        if inherited_codex is not None:
            env["CODEX_EXE"] = str(inherited_codex)
        env["PATH"] = str(temp_path) + os.pathsep + env.get("PATH", "")
        return subprocess.run(
            arguments,
            cwd=temp_path,
            env=env,
            capture_output=True,
            encoding="utf-8",
            errors="replace",
            timeout=30,
            check=False,
        )

    def test_path_discovery_does_not_pin_codex_exe(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            env_path = temp_path / ".env"
            _ = env_path.write_text("SENTINEL=keep\nCODEX_EXE=\n", encoding="utf-8")
            completed = self._run_installer(temp_path)
            codex_exe = _env_value(env_path, "CODEX_EXE")
            sentinel = _env_value(env_path, "SENTINEL")

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertEqual(codex_exe, "")
        self.assertEqual(sentinel, "keep")
        self.assertIn("PATH-discovered Codex command was not saved", output)

    def test_explicit_codex_exe_is_persisted(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            explicit_codex = temp_path / "explicit-codex"
            _ = explicit_codex.write_text("#!/usr/bin/env sh\nexit 0\n", encoding="utf-8")
            explicit_codex.chmod(explicit_codex.stat().st_mode | stat.S_IXUSR)
            completed = self._run_installer(
                temp_path,
                explicit_codex=explicit_codex,
            )
            codex_exe = _env_value(temp_path / ".env", "CODEX_EXE")

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertEqual(codex_exe, str(explicit_codex))

    def test_inherited_codex_exe_does_not_become_a_persistent_pin(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            inherited_codex = temp_path / "inherited-codex"
            _ = inherited_codex.write_text("#!/usr/bin/env sh\nexit 0\n", encoding="utf-8")
            inherited_codex.chmod(inherited_codex.stat().st_mode | stat.S_IXUSR)
            completed = self._run_installer(
                temp_path,
                inherited_codex=inherited_codex,
            )
            codex_exe = _env_value(temp_path / ".env", "CODEX_EXE")

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertEqual(codex_exe, "")
        self.assertIn("Inherited CODEX_EXE was not saved", output)


if __name__ == "__main__":
    _ = unittest.main()
