"""Run the real cutover outer transaction, including catch and failure publication."""
import unittest
import test_maintenance_publication as fixtures


class CutoverOuterFailureTests(unittest.TestCase):
    run_case = fixtures.MaintenancePublicationTests.run_case

    def test_competing_restart_after_observation_survives_outer_catch(self):
        self.run_case(r'''
$ModePath=Join-Path $RepoRoot '.codex_discord_runtime'
$CutoverStatePath=Join-Path $RepoRoot '.codex_discord_runtime.cutover'
$RustHeartbeat=Join-Path $RepoRoot 'heartbeat'
$CutoverIdentity='1|99';$DryRun=$false;$Runtime='rust'
$ObserveSeconds=0;$HeartbeatMaxAgeSeconds=45;$HeartbeatBootstrapGraceSeconds=0
[IO.File]::WriteAllText($ModePath,'python')
function Assert-RustCutoverPreflight {}
function Backup-Store {}
function Get-PythonIdentity {''}
$script:starts=0;$script:alive=$false
$script:birth=[datetime]::UtcNow.AddSeconds(-2)
function Get-VerifiedRustProcess {if($script:alive){[pscustomobject]@{Id=77;StartTime=$script:birth}}}
function Start-Rust {
 $script:starts++;$script:alive=$true
 [IO.File]::WriteAllText($RustHeartbeat,"pid=77`nupdated_at=$([DateTimeOffset]::UtcNow.ToUnixTimeSeconds())`n")
}
$realWait=(Get-Command Wait-RustHealthy).ScriptBlock
function Wait-RustHealthy {
 & $realWait
 $writerCode=@'
$ErrorActionPreference='Stop'
. (Join-Path $env:PUBLISH_SOURCE 'codex-discord-rust-control.ps1')
$lock=Enter-CdrControl $env:PUBLISH_ROOT
try{Write-NewCdrMarker (Join-Path $env:PUBLISH_ROOT '.codex_discord_rust.restart') 'competing-restart'}finally{$lock.Dispose()}
'@
 $encoded=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($writerCode))
 $writer=Start-Process powershell.exe -ArgumentList @('-NoProfile','-EncodedCommand',$encoded) -WindowStyle Hidden -PassThru -Wait
 if($writer.ExitCode -ne 0){throw 'writer failed'}
}
$begin=$source.LastIndexOf("`nRecover-InterruptedCutover`n")
try{$output=& ([scriptblock]::Create($source.Substring($begin)));throw 'outer entry accepted competing restart'}
catch{
 if($_.Exception.Message -notmatch 'Pending restart operation' -or $_.Exception.Message -notmatch 'recovery seal not published'){throw}
}
if($output -match 'cutover_complete'){throw 'false completion'}
if(Test-Path $DisablePath){throw 'outer catch resealed over competing restart'}
if([IO.File]::ReadAllText($RustRestart) -cne 'competing-restart'){throw 'restart changed'}
if($script:starts -ne 1 -or (Get-CutoverState).Phase -cne 'target_started'){throw 'cutover state lost'}
''')

    def test_recovered_source_completion_reports_actual_runtime(self):
        self.run_case(r'''
$CutoverStatePath=Join-Path $RepoRoot 'cutover';$ModePath=Join-Path $RepoRoot 'mode'
$CutoverIdentity='1|99';$DryRun=$false
$t=[pscustomobject]@{TransactionId='mine';OwnerIdentity='1|99';SourceRuntime='python';TargetRuntime='rust';Phase='target_healthy';TargetIdentity='77|99';CompletionRuntime='python'}
Set-CutoverPhase $t 'target_healthy';[IO.File]::WriteAllText($ModePath,'python')
function Get-PythonIdentity {'77|99'}
$output=Recover-InterruptedCutover
if($output -cne 'interrupted_cutover_finalized runtime=python'){throw 'wrong completed runtime displayed'}
if(Test-Path $CutoverStatePath){throw 'completion did not converge'}
''')

    def test_primary_error_survives_seal_control_lock_failure(self):
        self.run_case(r'''
$lock=Enter-CdrControl $RepoRoot
try{
 try{Throw-CutoverFailure ([pscustomobject]@{TransactionId='mine'}) 'ORIGINAL_FAILURE';throw 'failure hidden'}
 catch{if($_.Exception.Message -notmatch 'ORIGINAL_FAILURE' -or $_.Exception.Message -notmatch 'cdr_control_busy'){throw}}
 if(Test-Path $DisablePath){throw 'seal published without lock'}
}finally{$lock.Dispose()}
''')
