"""Actual engine reentry with stale/unwritable active state and independent audit."""
import unittest
import test_maintenance_v2_engine as fixture
from test_maintenance_completion_review import PRODUCTION_FAILURE_FIELDS


class MaintenanceCompletionAuditReentryTests(unittest.TestCase):
    run_case = fixture.MaintenanceV2EngineTests.run_case

    def test_active_only_first_and_later_failures_survive_without_event_audit(self):
        self.run_case(PRODUCTION_FAILURE_FIELDS + r'''
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
[void][IO.Directory]::CreateDirectory($auditPath)
$guard=[IO.File]::Open($DisablePath,'Open','Read','ReadWrite')
try{try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}}finally{$guard.Dispose()}
$first=(Read-CdrMaintenanceState $StatePath).FirstCompletionFailure.LastError
if(-not $first -or [IO.File]::Exists($auditPath)){throw 'first active-only failure not exercised'}
[IO.File]::WriteAllText($StopPath,$op)
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'stop intent ignored'}
catch{if($_.Exception.Message -notmatch 'post_launch_intent'){throw}}
$later=(Read-CdrMaintenanceState $StatePath).LastError
if($first -ceq $later){throw 'distinct second failure not exercised'}
[IO.File]::Delete($StopPath)
Invoke-CdrMaintenanceEngine $StatePath $op
$copy=Get-Content (Join-Path $s.Bundle 'completion-active-failure.json') -Raw -Encoding UTF8|ConvertFrom-Json
if((Test-Path $StatePath) -or $copy.Snapshot.LastError -cne $later -or $copy.First.LastError -cne $first){throw 'active-only first failure lost during cleanup'}
if($script:posts -ne 1 -or $script:starts -ne 1){throw 'active-only recovery replayed external work'}
''')

    def test_active_copy_is_separate_and_validated_even_when_active_becomes_empty(self):
        for mutation in ["$broken.Operation='f'*32", "$broken.Snapshot=$null",
                         "$broken.First='invalid'", "$broken.PSObject.Properties.Remove('First')"]:
            with self.subTest(mutation=mutation):
                self.run_case(PRODUCTION_FAILURE_FIELDS + r'''
$s.LastError='persisted only in active state'
Save-CdrCompletionFailureAudit $s -FromActiveState
if(Test-Path (Join-Path $s.Bundle 'completion-failure.json')){throw 'disk snapshot was promoted to a new failure event'}
$path=Join-Path $s.Bundle 'completion-active-failure.json'
$broken=Get-Content $path -Raw -Encoding UTF8|ConvertFrom-Json
MUTATION
Write-AtomicRestartMarker $path ($broken|ConvertTo-Json -Depth 10 -Compress)
$before=[Convert]::ToBase64String([IO.File]::ReadAllBytes($path))
$s.LastError=''
try{Save-CdrCompletionFailureAudit $s -FromActiveState;throw 'active copy validation bypassed'}
catch{if($_.Exception.Message -notmatch 'completion_failure_(owner_changed|snapshot_invalid)'){throw}}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($path)) -cne $before){throw 'invalid active copy overwritten'}
'''.replace('MUTATION', mutation))

    def test_clean_active_still_validates_independent_audit_identity_and_shape(self):
        for mutation in ["$broken.Operation='f'*32", "$broken.Latest=$null",
                         "$broken.ActiveStateUnpersisted='false'"]:
            with self.subTest(mutation=mutation):
                self.run_case(PRODUCTION_FAILURE_FIELDS + r'''
$s.LastError='independently captured error'
Save-CdrCompletionFailureAudit $s -ActiveStateUnpersisted
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
$broken=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
MUTATION
Write-AtomicRestartMarker $auditPath ($broken|ConvertTo-Json -Depth 10 -Compress)
$before=[Convert]::ToBase64String([IO.File]::ReadAllBytes($auditPath))
$s.LastError=''
try{Save-CdrCompletionFailureAudit $s;throw 'clean active bypassed audit validation'}
catch{if($_.Exception.Message -notmatch 'completion_failure_(owner_changed|snapshot_invalid)'){throw}}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($auditPath)) -cne $before){throw 'invalid audit was overwritten'}
'''.replace('MUTATION', mutation))

    def test_confirmed_failure_receipt_survives_stale_active_sending_state(self):
        self.run_case(PRODUCTION_FAILURE_FIELDS + r'''
$script:activeGuard=$null
function Invoke-RestMethod {
 $script:posts++
 $script:activeGuard=[IO.File]::Open($StatePath,'Open','Read','Read')
 [pscustomobject]@{id='12345';channel_id=$s.NotifyChannel}
}
$guard=[IO.File]::Open($DisablePath,'Open','Read','ReadWrite')
try{try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}}
finally{$guard.Dispose();if($script:activeGuard){$script:activeGuard.Dispose()}}
if((Read-CdrMaintenanceState $StatePath).FailureNoticePhase -cne 'sending'){throw 'post-result state-write failure not exercised'}
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
$before=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
if($before.Latest.FailureNoticePhase -cne 'sent' -or $before.Latest.FailureReceipt -cne '12345'){throw 'confirmed failure POST not archived'}
Invoke-CdrMaintenanceEngine $StatePath $op
$after=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
if($after.Latest.FailureNoticePhase -cne 'sent' -or $after.Latest.FailureReceipt -cne '12345'){throw 'confirmed failure receipt downgraded on cleanup'}
if((Test-Path $StatePath) -or $script:posts -ne 1){throw 'audit completion replayed POST or stayed blocked'}
''')

    def test_unknown_failure_result_survives_stale_active_sending_state(self):
        self.run_case(PRODUCTION_FAILURE_FIELDS + r'''
$script:activeGuard=$null
function Invoke-RestMethod {
 $script:posts++
 $script:activeGuard=[IO.File]::Open($StatePath,'Open','Read','Read')
 throw [TimeoutException]::new('fixture uncertainty after request')
}
$guard=[IO.File]::Open($DisablePath,'Open','Read','ReadWrite')
try{try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}}
finally{$guard.Dispose();if($script:activeGuard){$script:activeGuard.Dispose()}}
if((Read-CdrMaintenanceState $StatePath).FailureNoticePhase -cne 'sending'){throw 'unknown result state-write failure not exercised'}
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
$before=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
if($before.Latest.FailureNoticePhase -cne 'unknown' -or -not $before.Latest.FailureNoticeError){throw 'unknown result not archived'}
Invoke-CdrMaintenanceEngine $StatePath $op
$after=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
if($after.Latest.FailureNoticePhase -cne 'unknown' -or $after.Latest.FailureNoticeError -cne $before.Latest.FailureNoticeError){throw 'unknown failure result downgraded on cleanup'}
if((Test-Path $StatePath) -or $script:posts -ne 1){throw 'unknown audit cleanup replayed or stayed blocked'}
''')

    def test_conflicting_terminal_failure_evidence_is_never_overwritten(self):
        for first_phase, first_receipt, next_phase, next_receipt in [
                ('sent', '12345', 'unknown', ''),
                ('sent', '12345', 'sent', '67890'),
                ('unknown', '', 'sent', '12345')]:
            with self.subTest(first=first_phase, next=next_phase, receipt=next_receipt):
                self.run_case(PRODUCTION_FAILURE_FIELDS + r'''
$s.LastError='fixture completion failure';$s.FailureNoticePhase='FIRST_PHASE';$s.FailureReceipt='FIRST_RECEIPT'
Save-CdrCompletionFailureAudit $s
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
$before=[Convert]::ToBase64String([IO.File]::ReadAllBytes($auditPath))
$s.FailureNoticePhase='NEXT_PHASE';$s.FailureReceipt='NEXT_RECEIPT'
try{Save-CdrCompletionFailureAudit $s;throw 'conflicting terminal evidence accepted'}
catch{if($_.Exception.Message -notmatch 'failure_notice_conflict'){throw}}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($auditPath)) -cne $before){throw 'conflicting terminal evidence overwritten'}
'''.replace('FIRST_PHASE', first_phase).replace('FIRST_RECEIPT', first_receipt)
                   .replace('NEXT_PHASE', next_phase).replace('NEXT_RECEIPT', next_receipt))

    def test_active_delete_and_resave_failure_still_preserve_independent_audit(self):
        for entry_phase, previous_error in [('healthy', ''), ('healthy', 'previous preflight warning'),
                                            ('verified', ''), ('verified', 'previous preflight warning')]:
            with self.subTest(previous_error=previous_error, entry_phase=entry_phase):
                self.run_case(PRODUCTION_FAILURE_FIELDS + r'''
$s.LastError='PREVIOUS_ERROR';Save-CdrMaintenanceState $s $StatePath
if('ENTRY_PHASE' -eq 'verified'){
 Wait-CdrMaintenanceHeartbeats $s $StatePath
 $s|Add-Member RuntimeEvidence (Get-CdrRuntimeCompletionEvidence $s)
 $s|Add-Member InitialNotificationOutcome 'pending'
 $s.Phase='verified';Save-CdrMaintenanceState $s $StatePath
}
$script:activeGuard=$null
function Get-HeartbeatHealth {
 if((Test-Path ($StatePath+'.completed')) -and -not $script:activeGuard){
  $script:activeGuard=[IO.File]::Open($StatePath,'Open','Read','Read')
 }
 [pscustomobject]@{Healthy=$true;Bootstrap=$false}
}
$failure=''
try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{$failure=$_.Exception.Message}
finally{if($script:activeGuard){$script:activeGuard.Dispose()}}
if($failure -notmatch 'state preservation failed' -or (Test-Path $DisablePath) -or -not (Test-Path $StatePath)){throw 'R2-B real engine boundary not exercised'}
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
if(-not (Test-Path $auditPath)){throw 'R2-B independent audit skipped after active save error'}
$before=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
if(-not $before.Latest.LastError -or -not $failure.Contains($before.Latest.LastError) -or
   $before.Latest.LastError -ceq 'PREVIOUS_ERROR'){throw 'R2-B new cleanup error not archived'}
$errorText=$before.Latest.LastError
Invoke-CdrMaintenanceEngine $StatePath $op
$after=Get-Content $auditPath -Raw -Encoding UTF8|ConvertFrom-Json
if((Test-Path $StatePath) -or $after.Latest.LastError -cne $errorText){throw 'R2-B stale active erased independently preserved error'}
if($script:posts -ne 0 -or $script:starts -ne 1){throw 'R2-B recovery replayed external work'}
'''.replace('PREVIOUS_ERROR', previous_error).replace('ENTRY_PHASE', entry_phase))


if __name__ == '__main__':
    unittest.main()
