"""Pro R1/R2: real engine catch, notification files and failure POST adapter."""
import unittest
import test_maintenance_v2_engine as fixture
from maintenance_completion_fixture import CONNECTED
from maintenance_reconcile_fixture import RECONCILE

PRODUCTION_FAILURE_FIELDS = CONNECTED + r'''
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceFailure.ps1')
$s|Add-Member FailureNoticePhase 'none'
$s|Add-Member FailureReceipt ''
$s|Add-Member FailureNoticeError ''
Save-CdrMaintenanceState $s $StatePath
'''


class MaintenanceCompletionReviewTests(unittest.TestCase):
    run_case = fixture.MaintenanceV2EngineTests.run_case

    def test_korean_failure_evidence_uses_utf8_not_machine_default(self):
        self.run_case(PRODUCTION_FAILURE_FIELDS + r'''
$s.LastError='잠금 실패 — 한글 보존'
$PSDefaultParameterValues['Get-Content:Encoding']='Ascii'
Save-CdrCompletionFailureAudit $s
$saved=[IO.File]::ReadAllText((Join-Path $s.Bundle 'completion-failure.json'))|ConvertFrom-Json
if($saved.First.LastError -cne $s.LastError -or $saved.Latest.LastError -cne $s.LastError){throw 'Korean failure evidence corrupted'}
''')

    def test_notice_file_creation_failure_does_not_retain_ownership(self):
        self.run_case(PRODUCTION_FAILURE_FIELDS + r'''
[void][IO.Directory]::CreateDirectory((Get-CdrMaintenanceNoticePath $s))
$visible=@(Invoke-CdrMaintenanceEngine $StatePath $op 3>&1)
if((Test-Path $StatePath) -or (Test-Path $DisablePath) -or $script:posts -ne 0){throw 'notice creation failure affected runtime'}
if(($visible -join ' ') -notmatch 'notification_journal'){throw 'notice creation failure hidden'}
if(-not (Test-Path (Join-Path $s.Bundle 'notification-journal-error.json'))){throw 'notice creation failure not recorded'}
''')

    def test_notice_diagnostic_write_failure_warns_without_relocking_runtime(self):
        self.run_case(PRODUCTION_FAILURE_FIELDS + r'''
[void][IO.Directory]::CreateDirectory((Get-CdrMaintenanceNoticePath $s))
[void][IO.Directory]::CreateDirectory((Join-Path $s.Bundle 'notification-journal-error.json'))
$visible=@(Invoke-CdrMaintenanceEngine $StatePath $op 3>&1)
if((Test-Path $StatePath) -or (Test-Path $DisablePath) -or $script:posts -ne 0){throw 'notice diagnostic failure relocked runtime'}
if(($visible -join ' ') -notmatch 'failure evidence unavailable'){throw 'diagnostic storage failure was silent'}
$receipt=Get-Content ($StatePath+'.completed') -Raw|ConvertFrom-Json
if($receipt.InitialNotificationOutcome -cne 'pending'){throw 'unconfirmed notice advertised as sent'}
''')

    def test_legacy_completion_also_ignores_notice_only_file_failure(self):
        self.run_case(RECONCILE + r'''
$null=Initialize-CdrMaintenanceNotice $s 'unknown'
$guard=[IO.File]::Open((Get-CdrMaintenanceNoticePath $s),'Open','ReadWrite','None')
try{try{. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply}catch{}}
finally{$guard.Dispose()}
if((Test-Path $StatePath) -or (Test-Path $DisablePath)){throw 'R1 legacy: notice-only fault retained ownership'}
$receipt=Get-Content ($StatePath+'.completed') -Raw|ConvertFrom-Json
if($receipt.Reconciliation.PriorNotificationOutcome -cne 'unknown'){throw 'legacy uncertainty lost'}
if(-not (Test-Path (Join-Path $s.Bundle 'notification-journal-error.json'))){throw 'legacy notice file failure lost'}
if($script:posts -ne 0 -or $script:starts -ne 1){throw 'legacy completion replayed external work'}
''')

    def test_locked_notice_journal_cannot_hold_healthy_runtime_completion(self):
        self.run_case(PRODUCTION_FAILURE_FIELDS + r'''
$null=Initialize-CdrMaintenanceNotice $s
$path=Get-CdrMaintenanceNoticePath $s
$guard=[IO.File]::Open($path,'Open','ReadWrite','None')
try{try{$visible=@(Invoke-CdrMaintenanceEngine $StatePath $op 3>&1)}catch{}}
finally{$guard.Dispose()}
if((Test-Path $StatePath) -or (Test-Path $DisablePath)){throw 'R1: notice-only file fault retained runtime ownership'}
$receipt=Get-Content ($StatePath+'.completed') -Raw|ConvertFrom-Json
if($receipt.Phase -cne 'verified' -or $receipt.InitialNotificationOutcome -cne 'pending'){throw 'runtime proof/pending notice missing'}
$errorPath=Join-Path $s.Bundle 'notification-journal-error.json'
if(-not (Test-Path $errorPath)){throw 'notice journal error evidence missing'}
$errorRecord=Get-Content $errorPath -Raw|ConvertFrom-Json
if($errorRecord.Operation -cne $op -or $errorRecord.Status -cne 'unknown'){throw 'notice fault identity/outcome lost'}
if(($visible -join ' ') -notmatch 'notification_journal'){throw 'notice file failure was silent'}
Send-CdrCompletedNotice $receipt
if($script:posts -ne 0 -or $script:starts -ne 1){throw 'notice file fault caused POST or relaunch'}
if(@($script:calls|Where-Object{$_ -match ':(stop|install|cleanup)$'}).Count){throw 'runtime work replayed'}
''')

    def test_cleanup_failure_and_unknown_failure_post_survive_engine_reentry(self):
        self.run_case(PRODUCTION_FAILURE_FIELDS + r'''
function Invoke-RestMethod {$script:posts++;throw [TimeoutException]::new('fixture transport uncertainty')}
$guard=[IO.File]::Open($DisablePath,'Open','Read','ReadWrite')
try{try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'FAULT_NOT_INJECTED'}catch{if($_.Exception.Message -eq 'FAULT_NOT_INJECTED'){throw}}}
finally{$guard.Dispose()}
$failed=Read-CdrMaintenanceState $StatePath
if($failed.Phase -cne 'verified' -or -not $failed.LastError -or $failed.FailureNoticePhase -cne 'unknown' -or $script:posts -ne 1){throw 'actual engine/failure POST path not exercised'}
$originalError=$failed.LastError
Invoke-CdrMaintenanceEngine $StatePath $op
if((Test-Path $StatePath) -or (Test-Path $DisablePath)){throw 'owned cleanup did not finish'}
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
if(-not (Test-Path $auditPath)){throw 'R2: completion cleanup erased its failure evidence'}
$audit=Get-Content $auditPath -Raw|ConvertFrom-Json
if($audit.Operation -cne $op -or $audit.First.LastError -cne $originalError -or
   $audit.Latest.FailureNoticePhase -cne 'unknown' -or -not $audit.Latest.FailureNoticeError){throw 'first failure/unknown POST evidence lost'}
if($script:posts -ne 1 -or $script:starts -ne 1){throw 'cleanup reentry replayed POST or launch'}
''')

    def test_failure_audit_write_guard_preserves_active_evidence_until_retry(self):
        self.run_case(PRODUCTION_FAILURE_FIELDS + r'''
function Invoke-RestMethod {$script:posts++;throw [TimeoutException]::new('fixture uncertainty')}
$guard=[IO.File]::Open($DisablePath,'Open','Read','ReadWrite')
try{try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}}finally{$guard.Dispose()}
$failed=Read-CdrMaintenanceState $StatePath
if($failed.FailureNoticePhase -cne 'unknown'){throw 'failure adapter was not exercised'}
$originalError=$failed.LastError
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
$guard=[IO.File]::Open($auditPath,'Open','Read','None')
try{try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'AUDIT_LOCK_IGNORED'}catch{if($_.Exception.Message -eq 'AUDIT_LOCK_IGNORED'){throw}}}
finally{$guard.Dispose()}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'audit failure erased active evidence'}
if((Read-CdrMaintenanceState $StatePath).FirstCompletionFailure.LastError -cne $originalError){throw 'audit reentry overwrote first error'}
Invoke-CdrMaintenanceEngine $StatePath $op
$audit=Get-Content $auditPath -Raw|ConvertFrom-Json
if((Test-Path $StatePath) -or $audit.First.LastError -cne $originalError -or $audit.Latest.FailureNoticePhase -cne 'unknown'){throw 'audit retry lost original/unknown evidence'}
if($script:posts -ne 1 -or $script:starts -ne 1){throw 'audit retry repeated external work'}
''')

    def test_first_failure_is_immutable_when_latest_failure_changes(self):
        self.run_case(PRODUCTION_FAILURE_FIELDS + r'''
function Invoke-RestMethod {$script:posts++;throw [TimeoutException]::new('fixture uncertainty')}
$guard=[IO.File]::Open($DisablePath,'Open','Read','ReadWrite')
try{try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}}finally{$guard.Dispose()}
$failed=Read-CdrMaintenanceState $StatePath
$originalError=$failed.LastError
$failed.LastError='second cleanup failure';Save-CdrMaintenanceState $failed $StatePath
Invoke-CdrMaintenanceEngine $StatePath $op
$audit=Get-Content (Join-Path $s.Bundle 'completion-failure.json') -Raw|ConvertFrom-Json
$activeCopy=Get-Content (Join-Path $s.Bundle 'completion-active-failure.json') -Raw -Encoding UTF8|ConvertFrom-Json
if($audit.First.LastError -cne $originalError -or $activeCopy.Snapshot.LastError -cne 'second cleanup failure' -or
   $audit.Latest.FailureNoticePhase -cne 'unknown'){throw 'first/latest evidence contract lost'}
if($script:posts -ne 1){throw 'failure POST replayed'}
''')

if __name__ == '__main__':
    unittest.main()
