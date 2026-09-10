from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(os.name == "nt", "Windows tray contract")
class RustTrayContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="rust tray ")
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name) / "repo with spaces"
        self.repo.mkdir()
        for name in (
            "codex-discord-tray.ps1",
            "codex-discord-tray-restart-runtime.ps1",
            "codex-discord-atomic-file-runtime.ps1",
            "codex-discord-watchdog-identity-runtime.ps1",
            "codex-discord-tray-runtime.ps1",
        ):
            if (ROOT / name).exists():
                shutil.copyfile(ROOT / name, self.repo / name)
        self.lock = self.repo / ".codex_discord_rust.runtime.lock"
        self.lock.write_text("pid=42\n", encoding="utf-8")
        self.binary = self.repo / "target" / "release" / "cdr-runtime.exe"

    def run_ps(self, body: str, mode: str = "") -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["CODEX_DISCORD_RUNTIME"] = mode
        env["TRAY_CONTRACT_ROOT"] = str(self.repo)
        return subprocess.run(
            ["powershell.exe", "-NoProfile", "-Command", body],
            env=env, capture_output=True, encoding="utf-8", errors="replace",
            timeout=15, check=False,
        )

    def process_fixture(self, path: Path | None = None) -> str:
        actual = str(path or self.binary).replace("'", "''")
        return f"""
            $ErrorActionPreference = 'Stop'
            function Get-Process {{
                param($Id)
                if ($Id -eq 42) {{
                    [pscustomobject]@{{ Id=42; Path='{actual}';
                        StartTime=[datetime]'2026-09-01T00:00:00Z' }}
                }}
            }}
            function Get-CimInstance {{ [Console]::Error.WriteLine('LEGACY_SCAN'); $null }}
        """

    def test_default_tray_recognizes_only_the_bound_rust_process(self) -> None:
        # TRAY-1: default Rust selection must not search legacy Python processes.
        result = self.run_ps(self.process_fixture() + """
            & (Join-Path $env:TRAY_CONTRACT_ROOT 'codex-discord-tray.ps1') -Once
        """)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("running pid=42", result.stdout)
        self.assertNotIn("LEGACY_SCAN", result.stdout + result.stderr)

    def test_rust_selection_rejects_pid_path_mismatch_and_missing_lock(self) -> None:
        # TRAY-2: a reused PID / another checkout is not this bot.
        for missing in (False, True):
            with self.subTest(missing_lock=missing):
                if missing:
                    self.lock.unlink()
                result = self.run_ps(self.process_fixture(Path(self.temp.name) / "cdr-runtime.exe") + """
                    & (Join-Path $env:TRAY_CONTRACT_ROOT 'codex-discord-tray.ps1') -Once
                """, mode="rust")
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn("stopped", result.stdout)
                self.assertNotIn("LEGACY_SCAN", result.stdout + result.stderr)

    def test_runtime_file_selection_and_invalid_mode_have_no_legacy_fallback(self) -> None:
        (self.repo / ".codex_discord_runtime").write_text("rust\n", encoding="utf-8")
        body = self.process_fixture() + """
            & (Join-Path $env:TRAY_CONTRACT_ROOT 'codex-discord-tray.ps1') -Once
        """
        selected = self.run_ps(body)
        self.assertEqual(selected.returncode, 0, selected.stdout + selected.stderr)
        invalid = self.run_ps(body, mode="typo")
        self.assertNotEqual(invalid.returncode, 0)
        self.assertIn("Unsupported Codex Discord runtime selection", invalid.stderr)
        self.assertNotIn("LEGACY_SCAN", invalid.stdout + invalid.stderr)

    def restart_fixture(self) -> str:
        return self.process_fixture() + """
            $ScriptDir = $env:TRAY_CONTRACT_ROOT
            $RuntimeMode = 'rust'
            $RuntimeLockPath = Join-Path $ScriptDir '.codex_discord_rust.runtime.lock'
            $RestartRequestPath = Join-Path $ScriptDir '.codex_discord_bot.restart'
            $HeadlessLauncher = Join-Path $ScriptDir 'absent.vbs'
            $support = Join-Path $ScriptDir 'codex-discord-tray-runtime.ps1'
            if (Test-Path -LiteralPath $support) { . $support }
            . (Join-Path $ScriptDir 'codex-discord-tray-restart-runtime.ps1')
            function Get-CodexBotProcessIdentity { '42|99' }
            function Publish-AtomicTextFile { Write-Output 'LEGACY_MARKER' }
            function Get-ScheduledTask { $null }
            function Write-LauncherLog { param($Message); Write-Output $Message }
            Request-BotRestart
        """

    def test_restart_delegates_to_rust_identity_bound_protocol(self) -> None:
        # TRAY-3: use the Rust drain/probe/restart protocol; never publish old IPC markers.
        (self.repo / "codex-discord-rust-restart.ps1").write_text(
            "param([string]$RepoRoot, [string]$ExpectedBotIdentity)\n"
            "Write-Output \"RUST_RESTART root=$RepoRoot expected=$ExpectedBotIdentity\"\n",
            encoding="utf-8",
        )
        # PowerShell scripts can return successfully without setting LASTEXITCODE.
        for previous_exit in ("$null", "9"):
            with self.subTest(previous_exit=previous_exit):
                result = self.run_ps(f"$global:LASTEXITCODE = {previous_exit};\n" + self.restart_fixture())
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn(f"RUST_RESTART root={self.repo} expected=42|", result.stdout)
                self.assertNotIn("LEGACY_MARKER", result.stdout)

    def test_explicit_failed_exit_is_not_reported_as_queued(self) -> None:
        (self.repo / "codex-discord-rust-restart.ps1").write_text(
            "param([string]$RepoRoot, [string]$ExpectedBotIdentity)\nexit 7\n",
            encoding="utf-8",
        )
        result = self.run_ps(self.restart_fixture())
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Rust restart request failed with exit code 7", result.stderr)
        self.assertNotIn("tray_restart_requested", result.stdout)
        self.assertNotIn("LEGACY_MARKER", result.stdout)

    def test_restart_failure_remains_failure_without_legacy_fallback(self) -> None:
        # TRAY-4: keep the real failure visible and never switch transports.
        (self.repo / "codex-discord-rust-restart.ps1").write_text(
            "param([string]$RepoRoot, [string]$ExpectedBotIdentity)\n"
            "throw 'app-server readiness: controlled failure'\n", encoding="utf-8",
        )
        result = self.run_ps(self.restart_fixture())
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("app-server readiness: controlled failure", result.stderr)
        self.assertNotIn("LEGACY_MARKER", result.stdout)


if __name__ == "__main__":
    unittest.main()
