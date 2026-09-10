"""R1-R4: terminal ownership, complete initial publication, cutover boundaries."""
import unittest
import test_maintenance_recovery_safety as recovery
import test_maintenance_publication as cutover


class DeploymentThirdReviewTests(unittest.TestCase):
    run_case = recovery.MaintenanceRecoverySafetyTests.run_case

    def test_terminal_receipt_cannot_restart_dead_child_or_consume_new_intent(self):
        for markers in ('stop', 'disabled', 'both', 'none'):
            with self.subTest(markers=markers):
                self.run_case(r'''
Invoke-CdrDeploymentRecovery $statePath
$script:newAlive=$false
$before=[IO.File]::ReadAllText($statePath+'.completed')
''' + ("[IO.File]::WriteAllText($StopPath,'owned')\n" if markers in ('stop', 'both') else '')
                + ("[IO.File]::WriteAllText($DisablePath,'owned')\n" if markers in ('disabled', 'both') else '') + r'''
try{Invoke-CdrDeploymentRecovery $statePath;throw 'terminal operation launched again'}
catch{if($_.Exception.Message -notmatch 'completed deployment'){throw}}
if($script:starts -ne 1 -or (Test-Path ($statePath+'.launch'))){throw 'new launch authorized'}
if([IO.File]::ReadAllText($statePath+'.completed') -cne $before){throw 'receipt changed'}
''' + ("if([IO.File]::ReadAllText($StopPath) -cne 'owned'){throw 'stop lost'}\n" if markers in ('stop', 'both') else '')
                + ("if([IO.File]::ReadAllText($DisablePath) -cne 'owned'){throw 'disabled lost'}\n" if markers in ('disabled', 'both') else ''))

    def test_first_durable_deployment_journal_is_reentrant(self):
        self.run_case(r'''
$realSave=(Get-Command Save-CdrLaunchJournal).ScriptBlock
$script:failFirst=$true
function Save-CdrLaunchJournal {param($Path,$Record)
 & $realSave $Path $Record
 if($script:failFirst){$script:failFirst=$false;throw 'injected first durable save interruption'}
}
try{Invoke-CdrDeploymentRecovery $statePath;throw 'interruption hidden'}
catch{if($_.Exception.Message -notmatch 'injected first durable save'){throw}}
if($script:starts -ne 0 -or -not (Test-Path $DisablePath)){throw 'unsafe initial publication'}
Invoke-CdrDeploymentRecovery $statePath
if($script:starts -ne 1 -or (Test-Path ($statePath+'.launch'))){throw 'prepared did not converge'}
''')


class CutoverThirdReviewTests(unittest.TestCase):
    run_case = cutover.MaintenancePublicationTests.run_case

    def test_actual_cutover_heartbeat_rejects_future_and_prestart(self):
        self.run_case(r'''
$RustHeartbeat=Join-Path $RepoRoot 'heartbeat';$HeartbeatMaxAgeSeconds=45
$now=[DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
$process=[pscustomobject]@{Id=42;StartTime=[DateTimeOffset]::FromUnixTimeSeconds($now-5).UtcDateTime}
foreach($ts in @(($now+60),($now-10))){
 [IO.File]::WriteAllText($RustHeartbeat,"pid=42`nupdated_at=$ts`n")
 if((Get-VerifiedRustHeartbeatState $process) -eq 'healthy'){throw 'invalid timestamp certified'}
}
[IO.File]::WriteAllText($RustHeartbeat,"pid=42`nupdated_at=$now`n")
if((Get-VerifiedRustHeartbeatState $process) -ne 'healthy'){throw 'valid timestamp rejected'}
''')

    def test_pending_restart_blocks_cutover_before_publication(self):
        for phase in ('prepared', 'launching', 'child'):
            with self.subTest(phase=phase):
                self.run_case(r'''
$journal=Join-Path $RepoRoot '.codex_discord_rust.restart.launch'
$claim=Join-Path $RepoRoot '.codex_discord_rust.restart.claimed.42.99'
[IO.File]::WriteAllText($journal,'PHASE');[IO.File]::WriteAllText($claim,'owned')
try{Enter-CutoverMaintenance ([pscustomobject]@{TransactionId='other'});throw 'pending launch ignored'}
catch{if($_.Exception.Message -notmatch 'Pending restart operation'){throw}}
if(Test-Path $DisablePath){throw 'published competing maintenance'}
if([IO.File]::ReadAllText($journal) -cne 'PHASE' -or [IO.File]::ReadAllText($claim) -cne 'owned'){throw 'pending ownership changed'}
'''.replace('PHASE', phase))

    def test_real_cutover_entry_preserves_mode_and_pending_journal(self):
        for phase in ('prepared', 'launching', 'child'):
            with self.subTest(phase=phase):
                self.run_case(r'''
$ModePath=Join-Path $RepoRoot '.codex_discord_runtime'
$CutoverStatePath=Join-Path $RepoRoot '.codex_discord_runtime.cutover'
$CutoverIdentity='1|99';$Runtime='python';$DryRun=$false
$journal=Join-Path $RepoRoot '.codex_discord_rust.restart.launch'
$claim=Join-Path $RepoRoot '.codex_discord_rust.restart.claimed.42.99'
[IO.File]::WriteAllText($ModePath,'rust')
[IO.File]::WriteAllText($journal,'PHASE');[IO.File]::WriteAllText($claim,'owned')
function Assert-PythonRollbackPreflight {}
function Backup-StoreWithPython {}
function Get-VerifiedRustProcess {$null}
function Get-PythonIdentity {''}
function Invoke-PythonWatchdog {throw 'PYTHON START FORBIDDEN'}
function Start-Process {throw 'PROCESS START FORBIDDEN'}
$begin=$source.LastIndexOf("`nRecover-InterruptedCutover`n")
if($begin -lt 0){throw 'entry not located'}
try{& ([scriptblock]::Create($source.Substring($begin)));throw 'entry accepted unknown launch'}
catch{if($_.Exception.Message -notmatch 'Pending restart operation'){throw}}
if([IO.File]::ReadAllText($ModePath) -cne 'rust'){throw 'mode changed'}
if((Test-Path $CutoverStatePath) -or (Test-Path $DisablePath)){throw 'competing state published'}
if([IO.File]::ReadAllText($journal) -cne 'PHASE' -or [IO.File]::ReadAllText($claim) -cne 'owned'){throw 'ownership changed'}
'''.replace('PHASE', phase))

    def test_worker_publication_refuses_pending_launch_before_stopping(self):
        self.run_case(r'''
$source=Get-Content (Join-Path $env:PUBLISH_SOURCE 'scripts/Invoke-CdrDeployment.ps1') -Raw
$start=$source.IndexOf('    $control = Enter-CdrControl')
$end=$source.IndexOf("    Note 'normal_stop_requested'",$start)
$state=[pscustomobject]@{RepoRoot=$RepoRoot}
[IO.File]::WriteAllText((Join-Path $RepoRoot '.codex_discord_rust.restart.launch'),'launching')
try{& ([scriptblock]::Create($source.Substring($start,$end-$start)));throw 'worker ignored pending launch'}
catch{if($_.Exception.Message -notmatch 'Pending restart operation'){throw}}
if((Test-Path $DisablePath) -or (Test-Path $RustStop)){throw 'worker published stop'}
''')
