"""Actual recovery/control functions with deterministic external-process providers."""
from pathlib import Path
import hashlib
import os
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
LOAD = r'''
$ErrorActionPreference='Stop'
$RepoRoot=$env:SAFETY_ROOT; $BinaryPath=Join-Path $RepoRoot 'fixture.exe'
. (Join-Path $env:SAFETY_SOURCE 'codex-discord-rust-drain.ps1')
. (Join-Path $env:SAFETY_SOURCE 'codex-discord-rust-control.ps1')
. (Join-Path $env:SAFETY_SOURCE 'scripts/CdrDeploymentRecovery.ps1')
. (Join-Path $env:SAFETY_SOURCE 'scripts/CdrLaunchJournal.ps1')
$StopPath=Join-Path $RepoRoot '.codex_discord_rust.stop'
$DisablePath=Join-Path $RepoRoot '.codex_discord_bot.disabled'
$DrainPreparePath=Join-Path $RepoRoot '.codex_discord_rust.drain.prepare'
$DrainAckPath=Join-Path $RepoRoot '.codex_discord_rust.drain.ack'
$RestartPath=Join-Path $RepoRoot '.codex_discord_rust.restart'
[IO.File]::WriteAllText($BinaryPath,'hash fixture')
$statePath=Join-Path $RepoRoot 'state.json'
$state=[ordered]@{RepoRoot=$RepoRoot;BinaryPath=$BinaryPath;RuntimePid=42;RuntimeTicks='99';Marker='owned';
 BaselineHash='FIXTURE_HASH';CandidateHash='unused';LogPath=(Join-Path $RepoRoot 'log')}
[IO.File]::WriteAllText($statePath,($state|ConvertTo-Json))
[IO.File]::WriteAllText($StopPath,'owned');[IO.File]::WriteAllText($DisablePath,'owned')
$script:oldAlive=$false;$script:foreign=$false;$script:newAlive=$false;$script:starts=0;$script:waits=0
function Get-Process {
 param($Id,$ErrorAction)
 if($Id -eq 42 -and $script:oldAlive){[pscustomobject]@{Id=42;Path=$BinaryPath;StartTime=[datetime]::new(99,[DateTimeKind]::Utc)}}
 if($Id -eq 77 -and $script:newAlive){[pscustomobject]@{Id=77;Path=$BinaryPath;StartTime=[datetime]::new(99,[DateTimeKind]::Utc)}}
}
function Get-VerifiedRuntimeProcess {
 if($script:foreign){[pscustomobject]@{Id=88}}
 elseif($script:newAlive){[pscustomobject]@{Id=77}}
 elseif($script:oldAlive){[pscustomobject]@{Id=42}}
}
function Get-RustProcessIdentity {param($Process) if($Process){"$($Process.Id)|99"}else{''}}
function Get-VerifiedRuntimeIdentity {Get-RustProcessIdentity (Get-VerifiedRuntimeProcess)}
function Get-HeartbeatHealth {[pscustomobject]@{Healthy=$true;Bootstrap=$false}}
function Clear-DeadRuntimeArtifacts {if($script:oldAlive -or $script:foreign){throw 'live artifacts cleared'}}
function Start-RustRuntime {
 if($script:oldAlive -or $script:foreign){throw 'duplicate start'}
 if((Test-Path $StopPath) -or -not (Test-Path $DisablePath)){throw 'startup must retain maintenance seal without stop'}
 $script:starts++;$script:newAlive=$true
 Set-CdrLaunchStarting
 Set-CdrLaunchChild ([pscustomobject]@{Id=77;Path=$BinaryPath})
}
function Stop-VerifiedRuntime {throw 'KILL MUST NEVER BE CALLED'}
function Wait-RustRuntimeExit {$script:waits++;$script:oldAlive=$false}
'''.replace('FIXTURE_HASH', hashlib.sha256(b'hash fixture').hexdigest().upper())


class MaintenanceRecoverySafetyTests(unittest.TestCase):
    def run_case(self, body):
        with tempfile.TemporaryDirectory() as temp:
            result = subprocess.run(
                ['powershell.exe', '-NoProfile', '-Command', LOAD + body + '\nexit 0'],
                env={**os.environ, 'SAFETY_ROOT': temp, 'SAFETY_SOURCE': str(ROOT)},
                capture_output=True, encoding='utf-8', errors='replace', timeout=20)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_exit_then_start_once_and_repeat_verifies_receipt(self):
        self.run_case(r'''
$script:oldAlive=$true
Invoke-CdrDeploymentRecovery $statePath
Invoke-CdrDeploymentRecovery $statePath
if($script:waits -ne 1 -or $script:starts -ne 1){throw 'wrong handoff count'}
if(-not (Test-Path ($statePath+'.completed'))){throw 'missing completion receipt'}
''')

    def test_timeout_preserves_markers_and_never_runs_general_health_recovery(self):
        self.run_case(r'''
$script:oldAlive=$true
function Wait-RustRuntimeExit {throw 'controlled graceful_exit_timeout'}
function Get-HeartbeatHealth {throw 'generic health path was entered'}
try{Invoke-CdrDeploymentRecovery $statePath;throw 'timeout accepted'}
catch{if($_.Exception.Message -notmatch 'controlled graceful_exit_timeout'){throw}}
if($script:starts -ne 0){throw 'started on timeout'}
if([IO.File]::ReadAllText($DisablePath) -cne 'owned' -or [IO.File]::ReadAllText($StopPath) -cne 'owned'){throw 'markers changed'}
''')

    def test_foreign_live_instance_is_not_accepted_or_changed(self):
        self.run_case(r'''
$script:foreign=$true
try{Invoke-CdrDeploymentRecovery $statePath;throw 'foreign process accepted'}
catch{if($_.Exception.Message -notmatch 'Unrelated or unrecorded'){throw}}
if($script:starts -ne 0 -or -not (Test-Path $DisablePath)){throw 'foreign instance affected'}
''')

    def test_foreign_marker_is_preserved(self):
        self.run_case(r'''
[IO.File]::WriteAllText($StopPath,'foreign')
try{Invoke-CdrDeploymentRecovery $statePath;throw 'foreign marker accepted'}
catch{if($_.Exception.Message -notmatch 'Foreign maintenance marker'){throw}}
if([IO.File]::ReadAllText($StopPath) -cne 'foreign' -or $script:starts -ne 0){throw 'foreign marker affected'}
''')

    def test_create_only_publication_does_not_overwrite(self):
        self.run_case(r'''
try{Write-NewCdrMarker $StopPath 'replacement';throw 'overwrite accepted'}
catch{if($_.Exception.Message -eq 'overwrite accepted'){throw}}
if([IO.File]::ReadAllText($StopPath) -cne 'owned'){throw 'marker overwritten'}
''')

    def test_control_lock_blocks_other_owner_and_releases(self):
        self.run_case(r'''
$owner=Enter-CdrControl $RepoRoot
try {
 try{$other=Enter-CdrControl $RepoRoot; $other.Dispose(); throw 'lock ignored'}
 catch{if($_.Exception.Message -notmatch 'cdr_control_busy'){throw}}
}finally{$owner.Dispose()}
$next=Enter-CdrControl $RepoRoot; $next.Dispose()
''')

    def test_wrong_restart_receipt_never_claims_completion(self):
        self.run_case(r'''
[IO.File]::Delete($StopPath);[IO.File]::Delete($DisablePath)
$script:newAlive=$true
$fence=[pscustomobject]@{RuntimeId='old';ProcessIdentity='42|99';Nonce='owned'}
Write-CdrRestartCompletion $fence
if(-not (Test-CdrRestartCompleted $fence)){throw 'valid receipt rejected'}
$fence.Nonce='foreign'
if(Test-CdrRestartCompleted $fence){throw 'foreign receipt accepted'}
''')

    def test_completion_requires_real_heartbeat_not_bootstrap(self):
        self.run_case(r'''
$script:newAlive=$true
function Get-HeartbeatHealth {[pscustomobject]@{Healthy=$true;Bootstrap=$true}}
try{Wait-CdrReplacementReady -ExpectedIdentity '77|99' -TimeoutSeconds 0;throw 'bootstrap accepted'}
catch{if($_.Exception.Message -notmatch 'fresh matching heartbeat'){throw}}
''')

    def test_new_stop_intent_prevents_false_restart_completion(self):
        self.run_case(r'''
$script:newAlive=$true
$fence=[pscustomobject]@{RuntimeId='old';ProcessIdentity='42|99';Nonce='owned'}
Write-CdrRestartCompletion $fence
try{Test-CdrRestartCompleted $fence;throw 'active maintenance ignored'}
catch{if($_.Exception.Message -notmatch 'Another maintenance intent'){throw}}
''')

    def test_failed_start_reseals_maintenance_without_stopping_unknown_process(self):
        self.run_case(r'''
function Start-RustRuntime {$script:newAlive=$true;throw 'controlled startup uncertainty'}
try{Invoke-CdrDeploymentRecovery $statePath;throw 'failed launch accepted'}
catch{if($_.Exception.Message -notmatch 'controlled startup uncertainty'){throw}}
if([IO.File]::ReadAllText($DisablePath) -cne 'owned'){throw 'maintenance seal missing'}
if(Test-Path ($statePath+'.completed')){throw 'false completion receipt'}
try{Invoke-CdrDeploymentRecovery $statePath;throw 'unrecorded replacement accepted'}
catch{if($_.Exception.Message -notmatch 'Unrelated or unrecorded'){throw}}
''')


if __name__ == '__main__':
    unittest.main()
