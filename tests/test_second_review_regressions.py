"""Second Pro review: observable partial-failure and completion contracts."""
import unittest
import test_maintenance_recovery_safety as fixtures


class SecondReviewRegressions(unittest.TestCase):
    run_case = fixtures.MaintenanceRecoverySafetyTests.run_case
    def test_disabled_only_completes_authorized_stop_before_wait(self):
        self.run_case(r'''
$script:oldAlive=$true
[IO.File]::Delete($StopPath)
function Wait-RustRuntimeExit {
 if(-not [IO.File]::Exists($StopPath)){throw 'waiting without a stop request'}
 if([IO.File]::ReadAllText($StopPath) -cne 'owned'){throw 'wrong stop owner'}
 $script:waits++;$script:oldAlive=$false
}
Invoke-CdrDeploymentRecovery $statePath
if($script:starts -ne 1 -or $script:waits -ne 1){throw 'partial publication did not recover'}
''')

    def test_completed_deployment_rejects_new_owned_stop(self):
        self.run_case(r'''
Invoke-CdrDeploymentRecovery $statePath
[IO.File]::WriteAllText($StopPath,'owned')
try{Invoke-CdrDeploymentRecovery $statePath;throw 'new stop ignored'}
catch{if($_.Exception.Message -notmatch 'maintenance intent'){throw}}
if([IO.File]::ReadAllText($StopPath) -cne 'owned'){throw 'new stop consumed'}
''')

    def test_completed_deployment_rejects_new_restart_fence(self):
        self.run_case(r'''
Invoke-CdrDeploymentRecovery $statePath
[IO.File]::WriteAllText($RestartPath,'foreign')
try{Invoke-CdrDeploymentRecovery $statePath;throw 'new fence ignored'}
catch{if($_.Exception.Message -notmatch 'restart fence'){throw}}
if([IO.File]::ReadAllText($RestartPath) -cne 'foreign'){throw 'fence changed'}
''')

    def test_completed_restart_rejects_new_drain(self):
        self.run_case(r'''
[IO.File]::Delete($StopPath);[IO.File]::Delete($DisablePath)
$script:newAlive=$true
$fence=[pscustomobject]@{RuntimeId='old';ProcessIdentity='42|99';Nonce='owned'}
Write-CdrRestartCompletion $fence
[IO.File]::WriteAllText($DrainPreparePath,'foreign')
try{Test-CdrRestartCompleted $fence;throw 'new drain ignored'}
catch{if($_.Exception.Message -notmatch 'maintenance intent'){throw}}
''')

    def test_deployment_receipt_failure_resumes_same_recorded_child(self):
        self.run_case(r'''
$realAtomic=(Get-Command Write-AtomicRestartMarker).ScriptBlock
$script:failReceipt=$true
function Write-AtomicRestartMarker {
 param($Path,$Text)
 if($Path -eq ($statePath+'.completed') -and $script:failReceipt){
  $script:failReceipt=$false;throw 'injected completion write failure'
 }
 & $realAtomic -Path $Path -Text $Text
}
try{Invoke-CdrDeploymentRecovery $statePath;throw 'write failure hidden'}
catch{if($_.Exception.Message -notmatch 'injected completion write failure'){throw}}
if($script:starts -ne 1 -or -not (Test-Path ($statePath+'.launch'))){throw 'launch ownership lost'}
$script:CdrLaunchJournalPath=$null
Invoke-CdrDeploymentRecovery $statePath
if($script:starts -ne 1){throw 'duplicate replacement was started'}
if((Test-Path $DisablePath) -or (Test-Path ($statePath+'.launch'))){throw 'recovery did not converge'}
$receipt=[IO.File]::ReadAllText($statePath+'.completed')|ConvertFrom-Json
if($receipt.ReplacementIdentity -cne '77|99'){throw 'wrong child certified'}
''')
