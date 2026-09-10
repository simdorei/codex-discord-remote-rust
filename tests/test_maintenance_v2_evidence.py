import unittest
from test_maintenance_v2_engine import MaintenanceV2EngineTests


class MaintenanceV2EvidenceTests(unittest.TestCase):
    run_case = MaintenanceV2EngineTests.run_case

    def test_update_success_notice_reports_binary_proof_without_claiming_room_cleanup(self):
        self.run_case(r'''
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceNotification.ps1')
function Get-CdrMaintenanceChild {[pscustomobject]@{Id=777}}
$script:notice=''
function Send-CdrMaintenanceMessage($s,$content,$nonce,$timeout) {$script:notice=$content;return '12345'}
$s=Read-CdrMaintenanceState $StatePath
$receipt=Send-CdrMaintenanceResult $s $StatePath
if($receipt -cne '12345'){throw 'delivery receipt changed'}
if($script:notice.Contains('cleanup')){throw 'update-only ticket falsely claims mirror cleanup'}
if(-not $script:notice.Contains('PID=777') -or -not $script:notice.Contains('two fresh heartbeats') -or
   -not $script:notice.Contains($s.CandidateHash)){throw 'binary proof absent from update notice'}
''')

    def test_post_mutation_baseline_hash_is_refused_by_actual_artifact_guard(self):
        self.run_case(r'''
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceActions.ps1')
function Assert-CdrMaintenanceProgramPins {}
$EnvPath=Join-Path $RepoRoot '.env'
[IO.File]::WriteAllText($EnvPath,'fixture environment only')
$s=Read-CdrMaintenanceState $StatePath
[IO.File]::WriteAllText($BinaryPath,'baseline')
[IO.File]::WriteAllText($s.CandidatePath,'candidate')
[IO.File]::WriteAllText($s.OperatorPath,'operator')
$s.EnvHash=Get-CdrArtifactHash $EnvPath;$s.BaselineHash=Get-CdrArtifactHash $BinaryPath
$s.CandidateHash=Get-CdrArtifactHash $s.CandidatePath;$s.OperatorHash=Get-CdrArtifactHash $s.OperatorPath
$s.Phase='mutation_started'
try{Assert-CdrMaintenanceArtifacts $s;throw 'baseline accepted after mutation'}
catch{if($_.Exception.Message -notmatch 'installed_hash_wrong_for_phase'){throw}}
''')

    def test_modified_program_pin_is_rejected(self):
        self.run_case(r'''
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceState.ps1')
$s=Read-CdrMaintenanceState $StatePath
$pins=@(Get-CdrMaintenanceProgramPaths|ForEach-Object{
 $path=Join-Path $RepoRoot $_
 [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path))
 [IO.File]::WriteAllText($path,'fixture code')
 [pscustomobject]@{Path=$_;Hash=(Get-CdrArtifactHash $path)}
})
$s|Add-Member ProgramPins $pins
Assert-CdrMaintenanceProgramPins $s
[IO.File]::WriteAllText((Join-Path $RepoRoot 'scripts/CdrMaintenanceEngine.ps1'),'changed')
try{Assert-CdrMaintenanceProgramPins $s;throw 'changed code accepted'}
catch{if($_.Exception.Message -notmatch 'program_changed_after_arming'){throw}}
''')

    def test_unknown_failure_notification_is_recorded_and_not_resent(self):
        self.run_case(r'''
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceNotification.ps1')
$s=Read-CdrMaintenanceState $StatePath
$s|Add-Member FailureNoticePhase 'none';$s|Add-Member FailureReceipt '';$s|Add-Member FailureNoticeError ''
$script:posts=0
function Send-CdrMaintenanceMessage {$script:posts++;throw 'fixture transport uncertainty'}
Publish-CdrMaintenanceFailure $s $StatePath
$s=Read-CdrMaintenanceState $StatePath
Publish-CdrMaintenanceFailure $s $StatePath
if($script:posts -ne 1 -or $s.FailureNoticePhase -ne 'unknown'){throw 'failure notice replayed or lost'}
''')


if __name__ == '__main__':
    unittest.main()
