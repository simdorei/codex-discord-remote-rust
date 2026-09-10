from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parents[1]
ATOMIC_RUNTIME = ROOT / "codex-discord-atomic-file-runtime.ps1"
TRAY_RESTART_RUNTIME = ROOT / "codex-discord-tray-restart-runtime.ps1"
POWERSHELL = shutil.which("powershell.exe")

pytestmark = pytest.mark.skipif(
    os.name != "nt" or POWERSHELL is None,
    reason="PowerShell restart marker tests run on Windows",
)


def test_restart_marker_is_complete_when_a_concurrent_claimer_observes_it() -> None:
    with tempfile.TemporaryDirectory() as temp_dir:
        root = Path(temp_dir)
        marker = root / ".codex_discord_bot.restart"
        claim = root / ".codex_discord_bot.restart.claimed"
        expected = "identity=" + ("7" * (16 * 1024 * 1024)) + "|100"
        command = "\n".join(
            [
                "$ErrorActionPreference = 'Stop'",
                f". {str(ATOMIC_RUNTIME)!r}",
                f"$content = 'identity=' + ('7' * {16 * 1024 * 1024}) + '|100'",
                f"Publish-AtomicTextFile -Path {str(marker)!r} -Content $content",
            ]
        )
        process = subprocess.Popen(
            ["powershell.exe", "-NoProfile", "-Command", command],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            try:
                _ = marker.replace(claim)
                break
            except (FileNotFoundError, PermissionError):
                if process.poll() is not None and not marker.exists():
                    break
                time.sleep(0.0005)
        stdout, stderr = process.communicate(timeout=30)
        if not claim.exists() and marker.exists():
            _ = marker.replace(claim)

        assert process.returncode == 0, stdout + stderr
        assert claim.read_text(encoding="utf-8") == expected


def test_tray_restart_executes_bound_atomic_publish_and_reenables_task() -> None:
    with tempfile.TemporaryDirectory() as temp_dir:
        root = Path(temp_dir)
        marker = root / ".codex_discord_bot.restart"
        enabled = root / "enabled"
        started = root / "started"
        command = "\n".join(
            [
                "$ErrorActionPreference = 'Stop'",
                f"$BotScript = {str(root / 'codex_discord_bot.py')!r}",
                f"$RuntimeLockPath = {str(root / 'runtime.lock')!r}",
                f"$RestartRequestPath = {str(marker)!r}",
                f". {str(ATOMIC_RUNTIME)!r}",
                f". {str(TRAY_RESTART_RUNTIME)!r}",
                "function Get-CodexBotProcessIdentity { return '17|100' }",
                "function Write-LauncherLog { param([string]$Message) }",
                "function Get-ScheduledTask {",
                "    param([string]$TaskName, [object]$ErrorAction)",
                "    return [pscustomobject]@{ Settings = [pscustomobject]@{ Enabled = $false } }",
                "}",
                "function Enable-ScheduledTask {",
                "    param([string]$TaskName)",
                f"    [System.IO.File]::WriteAllText({str(enabled)!r}, $TaskName)",
                "}",
                "function Start-ScheduledTask {",
                "    param([string]$TaskName)",
                f"    [System.IO.File]::WriteAllText({str(started)!r}, $TaskName)",
                "}",
                "Request-BotRestart",
            ]
        )
        completed = subprocess.run(
            ["powershell.exe", "-NoProfile", "-Command", command],
            capture_output=True,
            encoding="utf-8",
            errors="replace",
            timeout=30,
            check=False,
        )

        output = completed.stdout + completed.stderr
        assert completed.returncode == 0, output
        assert marker.read_text(encoding="utf-8") == "identity=17|100"
        assert enabled.read_text(encoding="utf-8") == "Codex Discord Bot"
        assert started.read_text(encoding="utf-8") == "Codex Discord Bot"
