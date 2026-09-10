"""Actual launch/drain/action functions, with only external process/clock providers."""
import unittest
from test_maintenance_v2_engine import MaintenanceV2EngineTests

BOUNDARY = r'''
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceActions.ps1')
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceLaunch.ps1')
. (Join-Path $env:V2_SOURCE 'scripts/CdrLaunchJournal.ps1')
$StopPath=Join-Path $RepoRoot '.codex_discord_rust.stop'
$DisablePath=Join-Path $RepoRoot '.codex_discord_bot.disabled'
$DrainPreparePath=Join-Path $RepoRoot '.codex_discord_rust.drain.prepare'
$DrainAckPath=Join-Path $RepoRoot '.codex_discord_rust.drain.ack'
$DrainIdentityPath=Join-Path $RepoRoot '.codex_discord_rust.drain.identity'
$HeartbeatPath=Join-Path $RepoRoot '.codex_discord_rust.heartbeat'
$RestartPath=Join-Path $RepoRoot '.codex_discord_rust.restart'
$s=Read-CdrMaintenanceState $StatePath
$s.Phase='launch_ready';Save-CdrMaintenanceState $s $StatePath
[IO.File]::WriteAllText($DisablePath,$s.Operation)
$script:starts=0;$script:childAlive=$false
function Get-RustProcessIdentity {param($Process) if($Process){"$($Process.Id)|99"}else{''}}
function Get-Process {param($Id,$Name,$ErrorAction)
 if($script:childAlive -and ($Id -eq 77 -or $Name -eq 'cdr-runtime')){[pscustomobject]@{Id=77;Path=$BinaryPath}}
}
function Get-VerifiedRuntimeIdentity {if($script:childAlive){'77|99'}else{''}}
function Get-RuntimePid {0}
function Clear-DeadRuntimeArtifacts {}
function Invoke-CdrMaintenanceFullReadiness {Effect 'full_readiness'}
function Assert-CdrMaintenanceArtifacts {Effect 'artifacts'}
function Start-RustRuntime {
 $script:starts++;Set-CdrLaunchStarting;$script:childAlive=$true
 Set-CdrLaunchChild ([pscustomobject]@{Id=77;Path=$BinaryPath})
}
'''


class MaintenanceV2BoundaryTests(unittest.TestCase):
    run_case = MaintenanceV2EngineTests.run_case

    def test_recorded_child_exited_is_never_reset_or_relaunched(self):
        self.run_case(BOUNDARY + r'''
$j=New-CdrLaunchJournal (Join-Path $s.Bundle 'launch.json') ('maintenance:'+$s.Operation) $s.Fence $s.CandidateHash
$j.Phase='child';$j.ChildIdentity='77|99';Save-CdrLaunchJournal (Join-Path $s.Bundle 'launch.json') $j
try{Invoke-CdrMaintenanceLaunch $s $StatePath;throw 'dead child accepted'}catch{if($_.Exception.Message -notmatch 'recorded_child_dead'){throw}}
if($script:starts -ne 0 -or (Read-CdrLaunchJournal (Join-Path $s.Bundle 'launch.json')).Phase -ne 'child'){throw 'launch reset'}
''')

    def test_unrecorded_launch_outcome_stays_unknown(self):
        self.run_case(BOUNDARY + r'''
$j=New-CdrLaunchJournal (Join-Path $s.Bundle 'launch.json') ('maintenance:'+$s.Operation) $s.Fence $s.CandidateHash
$j.Phase='launching';Save-CdrLaunchJournal (Join-Path $s.Bundle 'launch.json') $j
try{Invoke-CdrMaintenanceLaunch $s $StatePath;throw 'unknown accepted'}catch{if($_.Exception.Message -notmatch 'launch_outcome_unknown'){throw}}
if($script:starts -ne 0){throw 'unknown relaunched'}
''')

    def test_actual_launch_once_then_adopts_only_recorded_child(self):
        self.run_case(BOUNDARY + r'''
Invoke-CdrMaintenanceLaunch $s $StatePath
Invoke-CdrMaintenanceLaunch $s $StatePath
if($script:starts -ne 1){throw 'duplicate launch'}
''')

    def test_foreign_marker_prevents_real_launch_boundary(self):
        self.run_case(BOUNDARY + r'''
[IO.File]::WriteAllText($StopPath,'foreign')
try{Invoke-CdrMaintenanceLaunch $s $StatePath;throw 'foreign accepted'}catch{if($_.Exception.Message -notmatch 'Foreign maintenance marker'){throw}}
if($script:starts -ne 0 -or [IO.File]::ReadAllText($StopPath) -cne 'foreign'){throw 'foreign affected'}
''')

    def test_old_process_liveness_blocks_launch(self):
        self.run_case(BOUNDARY + r'''
function Get-Process {param($Id,$Name,$ErrorAction) if($Id -eq 42){[pscustomobject]@{Id=42}}}
try{Invoke-CdrMaintenanceLaunch $s $StatePath;throw 'live old accepted'}catch{if($_.Exception.Message -notmatch 'original_PID_still_present'){throw}}
if($script:starts -ne 0){throw 'started before old exit'}
''')

    def test_stop_requires_real_matching_ack(self):
        self.run_case(BOUNDARY + r'''
. (Join-Path $env:V2_SOURCE 'codex-discord-rust-drain.ps1')
$s.Phase='stop_requested'
function Get-Process {param($Id,$Name,$ErrorAction) if($Id -eq 42){[pscustomobject]@{Id=42}}}
function Get-VerifiedRuntimeIdentity {'42|99'}
try{Invoke-CdrMaintenanceStop $s;throw 'missing ACK accepted'}catch{if($_.Exception.Message -notmatch 'fence changed'){throw}}
if(Test-Path $StopPath){throw 'stop published without ACK'}
''')

    def test_same_heartbeat_is_not_two_observations(self):
        self.run_case(BOUNDARY + r'''
Invoke-CdrMaintenanceLaunch $s $StatePath
$script:sleeps=0
function Get-HeartbeatHealth {[pscustomobject]@{Healthy=$true;Bootstrap=$false}}
[IO.File]::WriteAllText($HeartbeatPath,"pid=77`nupdated_at=123`n")
function Start-Sleep {$script:sleeps++;if($script:sleeps -gt 2){throw 'fixture_clock_end'}}
try{Wait-CdrMaintenanceHeartbeats $s $StatePath;throw 'same heartbeat accepted'}catch{if($_.Exception.Message -notmatch 'fixture_clock_end'){throw}}
if((Read-CdrMaintenanceState $StatePath).Heartbeats.Count){throw 'false heartbeat receipt'}
''')

    def test_two_increasing_heartbeats_are_saved(self):
        self.run_case(BOUNDARY + r'''
Invoke-CdrMaintenanceLaunch $s $StatePath
function Get-HeartbeatHealth {[pscustomobject]@{Healthy=$true;Bootstrap=$false}}
[IO.File]::WriteAllText($HeartbeatPath,"pid=77`nupdated_at=123`n")
function Start-Sleep {[IO.File]::WriteAllText($HeartbeatPath,"pid=77`nupdated_at=124`n")}
Wait-CdrMaintenanceHeartbeats $s $StatePath
if(((Read-CdrMaintenanceState $StatePath).Heartbeats -join ',') -cne '123,124'){throw 'heartbeat evidence missing'}
''')

    def test_saved_success_does_not_override_stale_live_heartbeat(self):
        self.run_case(BOUNDARY + r'''
Invoke-CdrMaintenanceLaunch $s $StatePath
$s.Phase='verified';$s.Heartbeats=@(1,2);$s.DiscordReceipt='12345';Save-CdrMaintenanceState $s $StatePath
function Get-HeartbeatHealth {[pscustomobject]@{Healthy=$false;Bootstrap=$false}}
try{Complete-CdrMaintenance $s $StatePath;throw 'stale completed'}catch{if($_.Exception.Message -notmatch 'heartbeat_not_fresh'){throw}}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'stale seal released'}
''')

    def test_new_stop_intent_cannot_be_consumed_by_completion(self):
        self.run_case(BOUNDARY + r'''
Invoke-CdrMaintenanceLaunch $s $StatePath
$s.Phase='verified';$s.Heartbeats=@(1,2);$s.DiscordReceipt='12345';Save-CdrMaintenanceState $s $StatePath
[IO.File]::WriteAllText($StopPath,$s.Operation)
function Get-HeartbeatHealth {[pscustomobject]@{Healthy=$true;Bootstrap=$false}}
try{Complete-CdrMaintenance $s $StatePath;throw 'new stop consumed'}catch{if($_.Exception.Message -notmatch 'post_launch_intent_present'){throw}}
if(-not (Test-Path $StopPath) -or -not (Test-Path $DisablePath)){throw 'intent lost'}
''')

    def test_readiness_returning_after_deadline_never_calls_launch(self):
        self.run_case(BOUNDARY + r'''
function Invoke-CdrMaintenanceFullReadiness {param($State);$State.Deadline=[DateTimeOffset]::UtcNow.AddSeconds(-1).ToString('o')}
try{Invoke-CdrMaintenanceLaunch $s $StatePath;throw 'launch after deadline'}catch{if($_.Exception.Message -notmatch 'deadline'){throw}}
if($script:starts -ne 0){throw 'expired launch called'}
''')


if __name__ == '__main__':
    unittest.main()
