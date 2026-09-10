"""First and current active-only errors must survive completion cleanup."""
import unittest
import test_maintenance_v2_engine as fixture
from test_maintenance_completion_review import PRODUCTION_FAILURE_FIELDS


class MaintenanceCompletionFirstCopyTests(unittest.TestCase):
    run_case = fixture.MaintenanceV2EngineTests.run_case

    def test_two_unwritable_archives_then_cleanup_preserves_both_errors_and_unknown(self):
        self.run_case(PRODUCTION_FAILURE_FIELDS + r'''
$auditPath=Join-Path $s.Bundle 'completion-failure.json'
$copyPath=Join-Path $s.Bundle 'completion-active-failure.json'
[void][IO.Directory]::CreateDirectory($auditPath)
[void][IO.Directory]::CreateDirectory($copyPath)
function Invoke-RestMethod {$script:posts++;throw [TimeoutException]::new('fixture unknown result')}
$guard=[IO.File]::Open($DisablePath,'Open','Read','ReadWrite')
try{try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}}finally{$guard.Dispose()}
$first=(Read-CdrMaintenanceState $StatePath).FirstCompletionFailure
if(-not $first.LastError -or -not $first.FailureObservation){throw 'first failure not observed'}
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'copy creation failure ignored'}catch{}
$active=Read-CdrMaintenanceState $StatePath
$second=$active.LastError
if($first.LastError -ceq $second -or $active.FailureNoticePhase -cne 'unknown'){throw 'distinct second failure or unknown not exercised'}
# Remove only these two empty fixture directories, not runtime/user files.
[IO.Directory]::Delete($auditPath)
[IO.Directory]::Delete($copyPath)
Invoke-CdrMaintenanceEngine $StatePath $op
$copy=Get-Content $copyPath -Raw -Encoding UTF8|ConvertFrom-Json
if((Test-Path $StatePath) -or $copy.Snapshot.LastError -cne $second -or
   ($copy.First|ConvertTo-Json -Depth 10 -Compress) -cne ($first|ConvertTo-Json -Depth 10 -Compress) -or
   $copy.Snapshot.FailureNoticePhase -cne 'unknown' -or
   $copy.Snapshot.FailureNoticeError -cne $active.FailureNoticeError){throw 'active-only first/current/unknown evidence lost'}
if($script:posts -ne 1 -or $script:starts -ne 1){throw 'recovery replayed external work'}
''')

    def test_first_only_evidence_is_copied_and_remains_immutable(self):
        self.run_case(PRODUCTION_FAILURE_FIELDS + r'''
$s.LastError='first error'
$s|Add-Member FirstCompletionFailure (Get-CdrCompletionFailureSnapshot $s)
$s.LastError=''
Save-CdrCompletionFailureAudit $s -FromActiveState
$path=Join-Path $s.Bundle 'completion-active-failure.json'
$copy=Get-Content $path -Raw -Encoding UTF8|ConvertFrom-Json
if($copy.First.LastError -cne 'first error'){throw 'first-only evidence not copied'}
$s.LastError='current error'
$s.FirstCompletionFailure=$null
Save-CdrCompletionFailureAudit $s -FromActiveState
$copy=Get-Content $path -Raw -Encoding UTF8|ConvertFrom-Json
if($copy.First.LastError -cne 'first error' -or $copy.Snapshot.LastError -cne 'current error'){throw 'first overwritten by empty state'}
$before=[Convert]::ToBase64String([IO.File]::ReadAllBytes($path))
$s.FirstCompletionFailure=Get-CdrCompletionFailureSnapshot $s
try{Save-CdrCompletionFailureAudit $s -FromActiveState;throw 'first conflict accepted'}
catch{if($_.Exception.Message -notmatch 'completion_failure_first_conflict'){throw}}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($path)) -cne $before){throw 'first conflict overwrote evidence'}
''')

    def test_no_first_record_can_be_enriched_by_the_first_real_failure(self):
        self.run_case(PRODUCTION_FAILURE_FIELDS + r'''
$s.LastError='older warning without first-completion evidence'
Save-CdrCompletionFailureAudit $s -FromActiveState
$path=Join-Path $s.Bundle 'completion-active-failure.json'
$copy=Get-Content $path -Raw -Encoding UTF8|ConvertFrom-Json
if(-not $copy.PSObject.Properties['First'] -or $null -ne $copy.First){throw 'missing first was invented'}
$s.LastError='first actual completion error'
$s|Add-Member FirstCompletionFailure (Get-CdrCompletionFailureSnapshot $s)
Save-CdrCompletionFailureAudit $s -FromActiveState
$copy=Get-Content $path -Raw -Encoding UTF8|ConvertFrom-Json
if($copy.First.LastError -cne $s.LastError -or $copy.Snapshot.LastError -cne $s.LastError){throw 'first was not enriched'}
''')


if __name__ == '__main__':
    unittest.main()
