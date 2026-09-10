"""Terminal control-flow and residual-claim guards, not only initial snapshots."""
import unittest
import test_maintenance_recovery_safety as recovery
import test_restart_transaction_recovery as restart
import test_maintenance_publication as cutover


class TerminalAndClaimTests(unittest.TestCase):
    run_case = recovery.MaintenanceRecoverySafetyTests.run_case

    def test_terminal_child_dies_between_lookups_without_new_authority(self):
        for markers in ('none', 'stop', 'disabled', 'both'):
            with self.subTest(markers=markers):
                self.run_case(r'''
Invoke-CdrDeploymentRecovery $statePath
$before=[IO.File]::ReadAllText($statePath+'.completed')
$script:terminalLookups=0
function Get-VerifiedRuntimeProcess {
 $script:terminalLookups++
 if($script:terminalLookups -eq 1){[pscustomobject]@{Id=77}}
 else{$script:newAlive=$false;$null}
}
function Clear-DeadRuntimeArtifacts {throw 'TERMINAL CLEANUP FORBIDDEN'}
''' + ("[IO.File]::WriteAllText($StopPath,'owned')\n" if markers in ('stop', 'both') else '')
                + ("[IO.File]::WriteAllText($DisablePath,'owned')\n" if markers in ('disabled', 'both') else '') + r'''
$script:caught=$false
try{$output=Invoke-CdrDeploymentRecovery $statePath}
catch{if($_.Exception.Message -match 'TERMINAL CLEANUP FORBIDDEN'){throw};$script:caught=$true}
if(-not $script:caught -or $output -contains 'recovery_completed_verified'){throw 'false terminal success'}
if($script:starts -ne 1 -or (Test-Path ($statePath+'.launch'))){throw 'new launch from terminal receipt'}
if([IO.File]::ReadAllText($statePath+'.completed') -cne $before){throw 'receipt changed'}
''' + ("if([IO.File]::ReadAllText($StopPath) -cne 'owned'){throw 'stop changed'}\n" if markers in ('stop', 'both') else '')
                + ("if([IO.File]::ReadAllText($DisablePath) -cne 'owned'){throw 'disabled changed'}\n" if markers in ('disabled', 'both') else ''))

    def test_deployment_refuses_claim_without_journal(self):
        self.run_case(r'''
$claim=Join-Path $RepoRoot '.codex_discord_rust.restart.claimed.42.99'
[IO.File]::WriteAllText($claim,'unknown-owner')
try{Invoke-CdrDeploymentRecovery $statePath;throw 'orphan claim accepted'}
catch{if($_.Exception.Message -notmatch 'Pending restart operation'){throw}}
if($script:starts -ne 0 -or [IO.File]::ReadAllText($claim) -cne 'unknown-owner'){throw 'orphan claim bypassed'}
if([IO.File]::ReadAllText($StopPath) -cne 'owned' -or [IO.File]::ReadAllText($DisablePath) -cne 'owned'){throw 'intent changed'}
''')

    def test_restart_receipt_refuses_orphan_claim(self):
        self.run_case(r'''
[IO.File]::Delete($StopPath);[IO.File]::Delete($DisablePath);$script:newAlive=$true
$f=[pscustomobject]@{RuntimeId='old';ProcessIdentity='42|99';Nonce='owned'}
Write-CdrRestartCompletion $f
$claim=Join-Path $RepoRoot '.codex_discord_rust.restart.claimed.42.99'
[IO.File]::WriteAllText($claim,'unknown-owner')
try{Test-CdrRestartCompleted $f;throw 'orphan claim certified'}
catch{if($_.Exception.Message -notmatch 'Pending restart operation'){throw}}
if([IO.File]::ReadAllText($claim) -cne 'unknown-owner'){throw 'claim changed'}
''')


class WatchdogClaimTests(unittest.TestCase):
    setUp = restart.RestartTransactionRecoveryTests.setUp
    run_boundary = restart.RestartTransactionRecoveryTests.run_boundary

    def test_generic_entry_refuses_orphan_claim_before_start_or_kill(self):
        self.run_boundary(r'''
foreach($p in @($RestartPath,$DrainPreparePath,$DrainAckPath)){[IO.File]::Delete($p)}
$claim=Join-Path $RepoRoot '.codex_discord_rust.restart.claimed.42.99'
[IO.File]::WriteAllText($claim,'unknown-owner')
try{& $entry;throw 'orphan claim ignored'}
catch{if($_.Exception.Message -notmatch 'Pending restart operation'){throw}}
if($script:starts -ne 0 -or [IO.File]::ReadAllText($claim) -cne 'unknown-owner'){throw 'orphan claim affected'}
''')


class CutoverRecoverySealTests(unittest.TestCase):
    run_case = cutover.MaintenancePublicationTests.run_case

    def test_interrupted_entry_does_not_reseal_over_pending_restart(self):
        for phase in ('prepared', 'launching', 'child'):
            for target in ('python', 'rust'):
                with self.subTest(phase=phase, target=target):
                    self.run_case(r'''
$ModePath=Join-Path $RepoRoot 'mode';$CutoverStatePath=Join-Path $RepoRoot 'cutover'
$CutoverIdentity='1|99';$DryRun=$false;$Runtime='TARGET'
$t=[pscustomobject]@{TransactionId='old';OwnerIdentity='1|99';SourceRuntime='python';TargetRuntime='rust';Phase='source_stopped'}
Set-CutoverPhase $t 'source_stopped';[IO.File]::WriteAllText($ModePath,'python')
$before=[IO.File]::ReadAllText($CutoverStatePath)
$journal=Join-Path $RepoRoot '.codex_discord_rust.restart.launch'
$claim=Join-Path $RepoRoot '.codex_discord_rust.restart.claimed.42.99'
[IO.File]::WriteAllText($journal,'PHASE');[IO.File]::WriteAllText($claim,'owner')
function Start-Rust {throw 'START FORBIDDEN'};function Stop-Rust {throw 'STOP FORBIDDEN'}
function Start-Python {throw 'START FORBIDDEN'};function Stop-Python {throw 'STOP FORBIDDEN'}
$begin=$source.LastIndexOf("`nRecover-InterruptedCutover`n")
$failureReason=$null
try{$entryOutput=& ([scriptblock]::Create($source.Substring($begin)))}
catch{$failureReason=$_.Exception.Message}
if([string]::IsNullOrWhiteSpace($failureReason)){throw 'pending operation returned success'}
if($failureReason -match 'START FORBIDDEN|STOP FORBIDDEN'){throw $failureReason}
if($Runtime -eq 'python'){
 if($failureReason -notmatch '^Cutover failed: Pending restart operation'){throw "unexpected source failure: $failureReason"}
}else{
 if($failureReason -notmatch '^Cutover failed: Interrupted cutover requires explicit source recovery'){throw "unexpected target failure: $failureReason"}
}
if($failureReason -notmatch 'recovery seal not published: Pending restart operation'){throw 'seal failure reason lost'}
if($entryOutput -match 'cutover_complete|interrupted_cutover_recovered|interrupted_cutover_finalized'){throw 'success output on refusal'}
if(Test-Path $DisablePath){throw 'unauthorized recovery seal published'}
if([IO.File]::ReadAllText($ModePath) -cne 'python' -or [IO.File]::ReadAllText($CutoverStatePath) -cne $before){throw 'cutover state changed'}
if([IO.File]::ReadAllText($journal) -cne 'PHASE' -or [IO.File]::ReadAllText($claim) -cne 'owner'){throw 'restart ownership changed'}
'''.replace('PHASE', phase).replace('TARGET', target))
