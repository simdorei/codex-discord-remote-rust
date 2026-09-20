. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceCommand.ps1')
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceBackup.ps1')
Write-CdrFixtureStage 'backup_modules_loaded'
$EnvPath=Join-Path $RepoRoot 'fixture.env'
$s=Read-CdrMaintenanceState $StatePath
Write-CdrFixtureStage 'state_loaded'
[IO.File]::WriteAllText($BinaryPath,'MZsynthetic baseline')
Write-CdrFixtureStage 'candidate_copy_start'
[IO.File]::Copy($env:CDR_TEST_RUNTIME_EXE,$s.CandidatePath,$false)
Write-CdrFixtureStage 'candidate_copied'
[IO.File]::WriteAllText($s.OperatorPath,'MZsynthetic operator; never executed')
Write-CdrFixtureStage 'artifact_hashes_start'
$s.BaselineHash=Get-CdrArtifactHash $BinaryPath;$s.CandidateHash=Get-CdrArtifactHash $s.CandidatePath
Write-CdrFixtureStage 'artifact_hashes_done'
Save-CdrMaintenanceState $s $StatePath
Write-CdrFixtureStage 'pre_stop_backup_start'
Invoke-CdrMaintenancePreStopBackup $s $StatePath
Write-CdrFixtureStage 'pre_stop_backup_done'
$saved=Read-CdrMaintenanceState $StatePath
Assert-CdrMaintenancePreStopBackup $saved
Write-CdrFixtureStage 'post_stop_backup_start'
New-CdrMaintenanceSnapshotReceipt $saved $StatePath 'PostStopBackup'
Write-CdrFixtureStage 'post_stop_backup_done'
$saved=Read-CdrMaintenanceState $StatePath
Assert-CdrMaintenanceBackupReceipt $saved 'PostStopBackup'
if($saved.PreStopBackup.SnapshotPath -ceq $saved.PostStopBackup.SnapshotPath){throw 'same backup reused'}
if($null -ne $saved.ActiveCommand){throw 'native receipt not committed'}
Write-CdrFixtureStage 'snapshot_complete'
Write-Output 'REAL_BACKUP_RECEIPTS_OK'
