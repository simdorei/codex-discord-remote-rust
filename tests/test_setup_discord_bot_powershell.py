from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from typing import TypedDict, cast


ROOT = Path(__file__).resolve().parents[1]


class TaskActionCapture(TypedDict):
    Execute: str
    Argument: str
    WorkingDirectory: str


class TaskTriggerCapture(TypedDict):
    Kind: str
    User: str
    IntervalSeconds: float


class TaskSettingsCapture(TypedDict):
    MultipleInstances: str
    StartWhenAvailable: bool
    AllowStartIfOnBatteries: bool
    DontStopIfGoingOnBatteries: bool
    RestartCount: int
    RestartIntervalSeconds: float


class TaskPrincipalCapture(TypedDict):
    UserId: str
    LogonType: str
    RunLevel: str


class ScheduledTaskCapture(TypedDict):
    Description: str
    TriggerCount: int


class TaskRegistrationCapture(TypedDict):
    TaskName: str
    Force: bool


class WatchdogCapture(TypedDict):
    Action: TaskActionCapture
    Triggers: list[TaskTriggerCapture]
    Settings: TaskSettingsCapture
    Principal: TaskPrincipalCapture
    Task: ScheduledTaskCapture
    Registration: TaskRegistrationCapture
    EnabledTaskName: str
    StartedTaskName: str


@unittest.skipUnless(
    os.name == "nt" and shutil.which("powershell.exe"), "Windows PowerShell test"
)
class SetupDiscordBotPowerShellTests(unittest.TestCase):
    def test_registers_elevated_minute_watchdog_without_console_focus_or_overlap(
        self,
    ) -> None:
        # Given
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            setup_script = temp_path / "setup-discord-bot.ps1"
            _ = setup_script.write_text(
                (ROOT / "setup-discord-bot.ps1").read_text(encoding="utf-8"),
                encoding="utf-8",
            )
            _ = (temp_path / "setup_discord_bot.py").write_text("", encoding="utf-8")
            _ = (temp_path / "codex-discord-watchdog.ps1").write_text(
                "", encoding="utf-8"
            )
            _ = (temp_path / "codex-discord-watchdog-hidden.vbs").write_text(
                "", encoding="utf-8"
            )
            fake_python = temp_path / "fake-python.cmd"
            _ = fake_python.write_text("@exit /b 0\n", encoding="utf-8")
            capture_path = temp_path / "scheduled-task.json"
            harness_path = temp_path / "harness.ps1"
            _ = harness_path.write_text(_POWERSHELL_HARNESS, encoding="utf-8")

            env = os.environ.copy()
            env["PYTHON_EXE"] = str(fake_python)
            env["HARNESS_SETUP"] = str(setup_script)
            env["HARNESS_CAPTURE"] = str(capture_path)
            # When
            completed = subprocess.run(
                [
                    "powershell.exe",
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    str(harness_path),
                ],
                cwd=temp_path,
                env=env,
                capture_output=True,
                text=True,
                timeout=30.0,
                check=False,
            )

            self.assertEqual(
                completed.returncode, 0, completed.stderr + completed.stdout
            )
            captured = cast(
                WatchdogCapture,
                json.loads(capture_path.read_text(encoding="utf-8-sig")),
            )

        # Then
        self.assertEqual(captured["Action"]["Execute"], "wscript.exe")
        self.assertEqual(captured["Action"]["WorkingDirectory"], str(temp_path))
        self.assertIn("//B //Nologo", captured["Action"]["Argument"])
        self.assertIn(
            str(temp_path / "codex-discord-watchdog-hidden.vbs"),
            captured["Action"]["Argument"],
        )
        self.assertEqual(
            [item["Kind"] for item in captured["Triggers"]], ["logon", "repeat"]
        )
        self.assertEqual(captured["Triggers"][1]["IntervalSeconds"], 60)
        self.assertEqual(captured["Settings"]["MultipleInstances"], "IgnoreNew")
        self.assertTrue(captured["Settings"]["StartWhenAvailable"])
        self.assertTrue(captured["Settings"]["AllowStartIfOnBatteries"])
        self.assertTrue(captured["Settings"]["DontStopIfGoingOnBatteries"])
        self.assertEqual(captured["Settings"]["RestartCount"], 3)
        self.assertEqual(captured["Settings"]["RestartIntervalSeconds"], 60)
        self.assertEqual(captured["Principal"]["RunLevel"], "Highest")
        self.assertEqual(captured["Principal"]["LogonType"], "Interactive")
        self.assertEqual(captured["Task"]["TriggerCount"], 2)
        self.assertEqual(
            captured["Registration"], {"TaskName": "Codex Discord Bot", "Force": True}
        )
        self.assertEqual(captured["EnabledTaskName"], "Codex Discord Bot")
        self.assertEqual(captured["StartedTaskName"], "Codex Discord Bot")


_POWERSHELL_HARNESS = r"""
$ErrorActionPreference = 'Stop'
$global:Captured = [ordered]@{ Triggers = @() }

function New-ScheduledTaskAction {
    param([string]$Execute, [string]$Argument, [string]$WorkingDirectory)
    $value = [pscustomobject]@{
        Execute = $Execute
        Argument = $Argument
        WorkingDirectory = $WorkingDirectory
    }
    $global:Captured.Action = $value
    return $value
}

function New-ScheduledTaskTrigger {
    param(
        [switch]$AtLogOn,
        [string]$User,
        [switch]$Once,
        [datetime]$At,
        [timespan]$RepetitionInterval,
        [timespan]$RepetitionDuration
    )
    $kind = if ($AtLogOn) { 'logon' } elseif ($Once) { 'repeat' } else { 'unknown' }
    $value = [pscustomobject]@{
        Kind = $kind
        User = $User
        IntervalSeconds = if ($Once) { $RepetitionInterval.TotalSeconds } else { 0 }
    }
    $global:Captured.Triggers = @($global:Captured.Triggers) + $value
    return $value
}

function New-ScheduledTaskSettingsSet {
    param(
        [string]$MultipleInstances,
        [switch]$StartWhenAvailable,
        [switch]$AllowStartIfOnBatteries,
        [switch]$DontStopIfGoingOnBatteries,
        [timespan]$ExecutionTimeLimit,
        [int]$RestartCount,
        [timespan]$RestartInterval
    )
    $value = [pscustomobject]@{
        MultipleInstances = $MultipleInstances
        StartWhenAvailable = $StartWhenAvailable.IsPresent
        AllowStartIfOnBatteries = $AllowStartIfOnBatteries.IsPresent
        DontStopIfGoingOnBatteries = $DontStopIfGoingOnBatteries.IsPresent
        RestartCount = $RestartCount
        RestartIntervalSeconds = $RestartInterval.TotalSeconds
    }
    $global:Captured.Settings = $value
    return $value
}

function New-ScheduledTaskPrincipal {
    param([string]$UserId, [string]$LogonType, [string]$RunLevel)
    $value = [pscustomobject]@{ UserId = $UserId; LogonType = $LogonType; RunLevel = $RunLevel }
    $global:Captured.Principal = $value
    return $value
}

function New-ScheduledTask {
    param($Action, [object[]]$Trigger, $Settings, $Principal, [string]$Description)
    $value = [pscustomobject]@{ Description = $Description; TriggerCount = $Trigger.Count }
    $global:Captured.Task = $value
    return $value
}

function Register-ScheduledTask {
    param([string]$TaskName, $InputObject, [switch]$Force)
    $global:Captured.Registration = [pscustomobject]@{
        TaskName = $TaskName
        Force = $Force.IsPresent
    }
    return $InputObject
}

function Start-ScheduledTask {
    param([string]$TaskName)
    $global:Captured.StartedTaskName = $TaskName
}

function Enable-ScheduledTask {
    param([string]$TaskName)
    $global:Captured.EnabledTaskName = $TaskName
}

& $env:HARNESS_SETUP | Out-Null
$global:Captured | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $env:HARNESS_CAPTURE -Encoding UTF8
"""


if __name__ == "__main__":
    _ = unittest.main()
