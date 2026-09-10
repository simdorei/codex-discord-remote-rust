import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class HeartbeatCompletionBoundaryTests(unittest.TestCase):
    def test_future_and_pre_start_heartbeat_are_not_completion_evidence(self):
        with tempfile.TemporaryDirectory() as temp:
            command = r'''
$ErrorActionPreference='Stop'
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $env:HEART_SOURCE 'codex-discord-rust-watchdog.ps1'),[ref]$tokens,[ref]$errors)
$fn=$ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Get-HeartbeatHealth'},$true)
Invoke-Expression $fn.Extent.Text
$HeartbeatPath=Join-Path $env:HEART_ROOT 'heartbeat'
$HealthHeartbeatStartupGraceSeconds=120;$HealthHeartbeatMaxAgeSeconds=45
$now=[DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
$process=[pscustomobject]@{Id=42;StartTime=[DateTimeOffset]::FromUnixTimeSeconds($now-5).UtcDateTime}
foreach($timestamp in @(($now+60),($now-10))){
 [IO.File]::WriteAllText($HeartbeatPath,"pid=42`nupdated_at=$timestamp`n")
 $health=Get-HeartbeatHealth $process
 if($health.Healthy -and -not $health.Bootstrap){throw 'invalid heartbeat certified completion'}
}
[IO.File]::WriteAllText($HeartbeatPath,"pid=42`nupdated_at=$now`n")
$health=Get-HeartbeatHealth $process
if(-not $health.Healthy -or $health.Bootstrap){throw 'current matching heartbeat rejected'}
exit 0
'''
            result = subprocess.run(['powershell.exe', '-NoProfile', '-Command', command],
                env={**os.environ, 'HEART_ROOT': temp, 'HEART_SOURCE': str(ROOT)},
                capture_output=True, encoding='utf-8', errors='replace', timeout=15)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
