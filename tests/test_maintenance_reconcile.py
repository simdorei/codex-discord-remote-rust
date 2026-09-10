"""M06/M07: actual completion-only entry, real source hashes/locks/files, fake OS processes only."""
import unittest
import test_maintenance_v2_engine as fixture
from maintenance_reconcile_fixture import RECONCILE


class MaintenanceReconcileTests(unittest.TestCase):
    run_case = fixture.MaintenanceV2EngineTests.run_case

    def test_existing_receipt_from_different_review_manifest_is_refused(self):
        self.run_case(RECONCILE + r'''
$forged=$s|ConvertTo-Json -Depth 12|ConvertFrom-Json
$forged.Phase='verified';$forged|Add-Member CompletionPolicy 'runtime-proof-v1'
$forged|Add-Member RuntimeEvidence ([pscustomobject]@{ChildIdentity=$manifest.ChildIdentity})
$forged|Add-Member Reconciliation ([pscustomobject]@{ManifestHash=('F'*64)})
Write-NewCdrMarker (Join-Path $s.Bundle 'completion.json') ($forged|ConvertTo-Json -Depth 12)
try{. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply;throw 'FAULT_NOT_REJECTED'}
catch{if($_.Exception.Message -notmatch 'reconcile_receipt_authority_mismatch'){throw}}
if(-not (Test-Path $DisablePath) -or -not (Test-Path $StatePath)){throw 'wrong manifest proof unsealed'}
''')

    def test_preflight_is_readonly_then_expired_legacy_completes_without_replay(self):
        self.run_case(RECONCILE + r'''
. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($StatePath)) -cne $originalBytes){throw 'preflight mutated original'}
if(-not (Test-Path $DisablePath) -or (Test-Path ($StatePath+'.completed'))){throw 'preflight finalized'}
. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply
if((Test-Path $StatePath) -or (Test-Path $DisablePath)){throw 'explicit cleanup incomplete'}
$receipt=Get-Content ($StatePath+'.completed') -Raw|ConvertFrom-Json
if($receipt.Reconciliation.OriginalStateHash -cne $manifest.OriginalStateHash -or $receipt.Phase -cne 'verified'){throw 'new proof missing'}
if($receipt.LastError -cne $s.LastError -or -not $receipt.Halted){throw 'historical error erased'}
if((Read-CdrMaintenanceNotice $s).Status -cne 'unknown'){throw 'unknown notice changed'}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes((Join-Path $originalRoot 'state.json'))) -cne $originalBytes){throw 'historical snapshot changed'}
. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply
if($script:starts -ne 1 -or $script:posts -ne 0){throw 'reconciliation replayed side effect'}
''')

    def test_partial_cleanup_preserves_original_hash_and_resumes(self):
        self.run_case(RECONCILE + r'''
$guard=[IO.File]::Open($StatePath,'Open','Read','Read')
try{try{. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply;throw 'fault absent'}catch{if($_.Exception.Message -eq 'fault absent'){throw}}}
finally{$guard.Dispose()}
if((Test-Path $DisablePath) -or -not (Test-Path $StatePath)){throw 'wrong partial boundary'}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($StatePath)) -cne $originalBytes){throw 'original state hash invalidated'}
. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply
if(Test-Path $StatePath){throw 'partial cleanup stuck'}
''')

    def test_changed_state_source_candidate_or_child_refuses_all_cleanup(self):
        cases = {
            'state': "[IO.File]::AppendAllText($StatePath,' ')",
            'source': "[IO.File]::AppendAllText((Join-Path $RepoRoot 'scripts/CdrMaintenanceState.ps1'),'#changed')",
            'candidate': "[IO.File]::WriteAllText($BinaryPath,'changed')",
            'child': "$script:childStarted=$script:childStarted.AddSeconds(5)",
            'dead': "$script:childAlive=$false",
            'stop': "[IO.File]::WriteAllText($StopPath,$op)",
            'foreign_seal': "[IO.File]::WriteAllText($DisablePath,'another-operation')",
            'newer_receipt': "[IO.File]::WriteAllText(($StatePath+'.completed'),'{\"Operation\":\"newer-ticket\"}')",
        }
        expected = {'state': 'active_state_changed', 'source': 'reviewed_entry_source_changed',
                    'candidate': 'runtime_artifact_changed', 'child': 'child_identity_changed',
                    'dead': 'requires_one_runtime', 'stop': 'new_or_foreign_intent',
                    'foreign_seal': 'Foreign maintenance marker', 'newer_receipt': 'newer_completion_preserved'}
        for name, fault in cases.items():
            with self.subTest(name=name):
                self.run_case(RECONCILE + fault + r'''
try{. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply;throw 'invalid accepted'}
catch{if($_.Exception.Message -notmatch 'EXPECTED_ERROR'){throw}}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'invalid cleanup performed'}
'''.replace('EXPECTED_ERROR', expected[name]))

    def test_wrong_manifest_digest_is_rejected_before_loading_modules(self):
        self.run_case(RECONCILE + r'''
try{. $entry -ManifestPath $manifestPath -ManifestHash ('A'*64) -ExpectedOperation $op -Apply;throw 'digest accepted'}
catch{if($_.Exception.Message -notmatch 'manifest_hash_mismatch'){throw}}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'digest mismatch mutated state'}
''')

    def test_competing_control_owner_blocks_reconcile(self):
        self.run_case(RECONCILE + r'''
$owner=Enter-CdrControl $RepoRoot -MaintenanceV2
try{
 try{. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply;throw 'lock bypassed'}
 catch{if($_.Exception.Message -notmatch 'cdr_control_busy'){throw}}
}finally{$owner.Dispose()}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'competing owner affected'}
''')


if __name__ == '__main__':
    unittest.main()
