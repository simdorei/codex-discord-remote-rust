"""M02-M04/M07: real persisted boundaries, including Windows file-sharing failures."""
import unittest
import test_maintenance_v2_engine as fixture
from maintenance_completion_fixture import CONNECTED

PREPARED = CONNECTED + r'''
Wait-CdrMaintenanceHeartbeats $s $StatePath
$s|Add-Member RuntimeEvidence (Get-CdrRuntimeCompletionEvidence $s)
$null=Initialize-CdrMaintenanceNotice $s
$s.Phase='verified';Save-CdrMaintenanceState $s $StatePath
'''


class MaintenanceCompletionCrashTests(unittest.TestCase):
    run_case = fixture.MaintenanceV2EngineTests.run_case

    def test_corrupted_persisted_heartbeat_proof_is_refused(self):
        self.run_case(PREPARED + r'''
$s.RuntimeEvidence.Heartbeats=@();Save-CdrMaintenanceState $s $StatePath
try{Complete-CdrMaintenance $s $StatePath;throw 'FAULT_NOT_REJECTED'}
catch{if($_.Exception.Message -notmatch 'maintenance_evidence_heartbeat'){throw}}
if(-not (Test-Path $DisablePath) -or -not (Test-Path $StatePath)){throw 'corrupt proof unsealed'}
''')

    def test_receipt_write_failure_preserves_owned_seal_then_resumes(self):
        self.run_case(PREPARED + r'''
$latest=$StatePath+'.completed'
[IO.File]::WriteAllText($latest,'{"Operation":"older-ticket"}')
$s.PreviousCompletedHash=Get-CdrArtifactHash $latest;Save-CdrMaintenanceState $s $StatePath
$guard=[IO.File]::Open($latest,'Open','Read','Read')
try {try{Complete-CdrMaintenance $s $StatePath;throw 'failure missing'}catch{if($_.Exception.Message -eq 'failure missing'){throw}}}
finally{$guard.Dispose()}
if(-not (Test-Path $DisablePath) -or -not (Test-Path $StatePath)){throw 'failed receipt released ownership'}
Invoke-CdrMaintenanceEngine $StatePath $op
if((Test-Path $DisablePath) -or (Test-Path $StatePath)){throw 'owned cleanup did not resume'}
if($script:posts -ne 0 -or $script:starts -ne 1){throw 'reentry replayed side effect'}
''')

    def test_active_delete_failure_after_unseal_resumes_despite_halted_expiry(self):
        self.run_case(PREPARED + r'''
$s.CreatedAt=$now.AddMinutes(-40).ToString('o');$s.Deadline=$now.AddMinutes(-10).ToString('o');$s.Halted=$true
Save-CdrMaintenanceState $s $StatePath
$guard=[IO.File]::Open($StatePath,'Open','Read','Read')
try {try{Complete-CdrMaintenance $s $StatePath;throw 'failure missing'}catch{if($_.Exception.Message -eq 'failure missing'){throw}}}
finally{$guard.Dispose()}
if((Test-Path $DisablePath) -or -not (Test-Path $StatePath) -or -not (Test-Path ($StatePath+'.completed'))){throw 'wrong crash boundary'}
Invoke-CdrMaintenanceEngine $StatePath $op
if(Test-Path $StatePath){throw 'expired terminal cleanup stuck'}
if($script:posts -ne 0 -or $script:starts -ne 1){throw 'reentry replayed side effect'}
''')

    def test_missing_seal_without_receipt_is_not_completion(self):
        self.run_case(PREPARED + r'''
[IO.File]::Delete($DisablePath)
try{Complete-CdrMaintenance $s $StatePath;throw 'unproven accepted'}catch{if($_.Exception.Message -notmatch 'without_completion'){throw}}
if(-not (Test-Path $StatePath)){throw 'unproven active ownership removed'}
''')

    def test_new_stop_same_owner_is_never_removed(self):
        self.run_case(PREPARED + r'''
[IO.File]::WriteAllText($StopPath,$op)
try{Complete-CdrMaintenance $s $StatePath;throw 'stop accepted'}catch{if($_.Exception.Message -notmatch 'post_launch_intent'){throw}}
if(-not (Test-Path $StopPath) -or -not (Test-Path $DisablePath)){throw 'stop/lock lost'}
''')

    def test_dead_stale_duplicate_or_wrong_artifact_refuses_cleanup(self):
        cases = {
            'dead': "$script:childAlive=$false",
            'stale': "function Get-HeartbeatHealth {[pscustomobject]@{Healthy=$false;Bootstrap=$false}}",
            'duplicate': "function Get-Process {param($Id,$Name,$ErrorAction) if($Name){@([pscustomobject]@{Id=77;Path=$BinaryPath},[pscustomobject]@{Id=88;Path=$BinaryPath})}elseif($Id -eq 77){[pscustomobject]@{Id=77;Path=$BinaryPath}}}",
            'artifact': "[IO.File]::WriteAllText($BinaryPath,'changed')",
        }
        for name, fault in cases.items():
            with self.subTest(name=name):
                self.run_case(PREPARED + fault + r'''
try{Complete-CdrMaintenance $s $StatePath;throw 'invalid proof accepted'}catch{if($_.Exception.Message -eq 'invalid proof accepted'){throw}}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'invalid proof unsealed'}
''')

    def test_timeout_preserved_without_second_post(self):
        self.run_case(CONNECTED + r'''
function Invoke-RestMethod {$script:posts++;throw [TimeoutException]::new('fixture timeout')}
Invoke-CdrMaintenanceEngine $StatePath $op
$notice=Read-CdrMaintenanceNotice $s
if($notice.Status -cne 'unknown' -or (Test-Path $DisablePath) -or (Test-Path $StatePath)){throw 'timeout affected runtime completion'}
$s=Get-Content ($StatePath+'.completed') -Raw|ConvertFrom-Json
Send-CdrCompletedNotice $s
if($script:posts -ne 1 -or $script:starts -ne 1){throw 'unknown notice replayed'}
''')

    def test_post_succeeded_but_result_write_failed_is_not_replayed(self):
        self.run_case(CONNECTED + r'''
$script:noticeGuard=$null
function Invoke-RestMethod {
 $script:posts++
 $script:noticeGuard=[IO.File]::Open((Get-CdrMaintenanceNoticePath $s),'Open','Read','Read')
 [pscustomobject]@{id='12345';channel_id=$s.NotifyChannel}
}
try{Invoke-CdrMaintenanceEngine $StatePath $op}finally{if($script:noticeGuard){$script:noticeGuard.Dispose()}}
if((Read-CdrMaintenanceNotice $s).Status -cne 'sending'){throw 'sending intent not preserved'}
$s=Get-Content ($StatePath+'.completed') -Raw|ConvertFrom-Json
Send-CdrCompletedNotice $s
if($script:posts -ne 1 -or (Test-Path $StatePath) -or (Test-Path $DisablePath)){throw 'ambiguous result replayed/relocked'}
''')

    def test_late_notice_never_overwrites_another_completion(self):
        self.run_case(CONNECTED + r'''
function Invoke-RestMethod {
 $script:posts++
 [IO.File]::WriteAllText(($StatePath+'.completed'),'{"Operation":"newer-ticket"}')
 [pscustomobject]@{id='12345';channel_id=$s.NotifyChannel}
}
Invoke-CdrMaintenanceEngine $StatePath $op
if((Get-Content ($StatePath+'.completed') -Raw|ConvertFrom-Json).Operation -cne 'newer-ticket'){throw 'late result overwrote newer proof'}
if((Read-CdrMaintenanceNotice $s).Receipt -cne '12345'){throw 'own notice receipt missing'}
''')


if __name__ == '__main__':
    unittest.main()
