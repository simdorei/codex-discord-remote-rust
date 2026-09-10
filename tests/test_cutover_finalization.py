"""Final publication must recheck identity and competing intent under the lock."""
import unittest
import test_maintenance_publication as fixtures


class CutoverFinalizationTests(unittest.TestCase):
    run_case = fixtures.MaintenancePublicationTests.run_case

    def test_finalization_refuses_stop_published_after_observation(self):
        self.run_case(r'''
$CutoverStatePath=Join-Path $RepoRoot 'cutover'
$ModePath=Join-Path $RepoRoot 'mode';$CutoverIdentity='1|99'
$t=[pscustomobject]@{TransactionId='mine';OwnerIdentity='1|99';SourceRuntime='python';TargetRuntime='rust';Phase='target_started';TargetIdentity='77|99'}
Set-CutoverPhase $t 'target_started'
[IO.File]::WriteAllText($ModePath,'rust')
function Get-RustIdentity {'77|99'}
function Test-TargetHealthy {$true}
# A separate OS process takes the actual common control lock after observation.
$writerCode=@'
$ErrorActionPreference='Stop'
. (Join-Path $env:PUBLISH_SOURCE 'codex-discord-rust-control.ps1')
$writer=Enter-CdrControl $env:PUBLISH_ROOT
try{Write-NewCdrMarker (Join-Path $env:PUBLISH_ROOT '.codex_discord_rust.stop') 'new-owner'}finally{$writer.Dispose()}
'@
$encoded=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($writerCode))
$writer=Start-Process powershell.exe -ArgumentList @('-NoProfile','-EncodedCommand',$encoded) -WindowStyle Hidden -PassThru -Wait
if($writer.ExitCode -ne 0){throw 'writer failed'}
$before=[IO.File]::ReadAllText($CutoverStatePath)
try{Complete-CutoverState $t;throw 'new stop ignored at completion'}
catch{if($_.Exception.Message -notmatch 'maintenance intent'){throw}}
if([IO.File]::ReadAllText($CutoverStatePath) -cne $before){throw 'state consumed despite stop'}
if([IO.File]::ReadAllText($RustStop) -cne 'new-owner'){throw 'stop consumed'}
''')

    def test_finalization_rejects_changed_identity(self):
        self.run_case(r'''
$CutoverStatePath=Join-Path $RepoRoot 'cutover';$ModePath=Join-Path $RepoRoot 'mode'
$CutoverIdentity='1|99'
$t=[pscustomobject]@{TransactionId='mine';OwnerIdentity='1|99';SourceRuntime='python';TargetRuntime='rust';Phase='target_started';TargetIdentity='77|99'}
Set-CutoverPhase $t 'target_started';[IO.File]::WriteAllText($ModePath,'rust')
function Get-RustIdentity {'88|99'}
function Test-TargetHealthy {$true}
try{Complete-CutoverState $t;throw 'replacement identity accepted'}
catch{if($_.Exception.Message -notmatch 'identity'){throw}}
if(-not (Test-Path $CutoverStatePath)){throw 'state consumed'}
''')

    def test_completion_and_interrupted_completion_use_persisted_identity(self):
        self.run_case(r'''
$CutoverStatePath=Join-Path $RepoRoot 'cutover';$ModePath=Join-Path $RepoRoot 'mode'
$CutoverIdentity='1|99';$DryRun=$false
$t=[pscustomobject]@{TransactionId='mine';OwnerIdentity='1|99';SourceRuntime='python';TargetRuntime='rust';Phase='target_started';TargetIdentity='77|99';CompletionRuntime='rust'}
Set-CutoverPhase $t 'target_healthy';[IO.File]::WriteAllText($ModePath,'rust')
function Get-RustIdentity {'77|99'}
function Test-TargetHealthy {$true}
Recover-InterruptedCutover
if(-not $script:CutoverRecoveryHandled -or (Test-Path $CutoverStatePath)){throw 'positive recovery did not complete'}
''')

    def test_interrupted_completion_refuses_restart(self):
        self.run_case(r'''
$CutoverStatePath=Join-Path $RepoRoot 'cutover';$ModePath=Join-Path $RepoRoot 'mode'
$CutoverIdentity='1|99';$DryRun=$false
$t=[pscustomobject]@{TransactionId='mine';OwnerIdentity='1|99';SourceRuntime='python';TargetRuntime='rust';Phase='target_started';TargetIdentity='77|99';CompletionRuntime='rust'}
Set-CutoverPhase $t 'target_healthy';[IO.File]::WriteAllText($ModePath,'rust')
function Get-RustIdentity {'77|99'}
function Test-TargetHealthy {$true}
Write-NewCdrMarker $RustRestart 'new-fence'
try{Recover-InterruptedCutover;throw 'interrupted completion ignored restart'}
catch{if($_.Exception.Message -notmatch 'Pending restart operation'){throw}}
if(-not (Test-Path $CutoverStatePath) -or [IO.File]::ReadAllText($RustRestart) -cne 'new-fence'){throw 'state or intent lost'}
''')
