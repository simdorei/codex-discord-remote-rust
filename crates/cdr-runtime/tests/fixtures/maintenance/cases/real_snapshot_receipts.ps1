. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceCommand.ps1')
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceBackup.ps1')
$EnvPath=Join-Path $RepoRoot 'fixture.env'
$s=Read-CdrMaintenanceState $StatePath
[IO.File]::WriteAllText($BinaryPath,'MZsynthetic baseline')
[IO.File]::Copy($env:CDR_TEST_RUNTIME_EXE,$s.CandidatePath,$false)
[IO.File]::WriteAllText($s.OperatorPath,'MZsynthetic operator; never executed')
$s.BaselineHash=Get-CdrArtifactHash $BinaryPath;$s.CandidateHash=Get-CdrArtifactHash $s.CandidatePath
Save-CdrMaintenanceState $s $StatePath
Invoke-CdrMaintenancePreStopBackup $s $StatePath
$saved=Read-CdrMaintenanceState $StatePath
Assert-CdrMaintenancePreStopBackup $saved
New-CdrMaintenanceSnapshotReceipt $saved $StatePath 'PostStopBackup'
$saved=Read-CdrMaintenanceState $StatePath
Assert-CdrMaintenanceBackupReceipt $saved 'PostStopBackup'
if($saved.PreStopBackup.SnapshotPath -ceq $saved.PostStopBackup.SnapshotPath){throw 'same backup reused'}
if($null -ne $saved.ActiveCommand){throw 'native receipt not committed'}
Write-Output 'REAL_BACKUP_RECEIPTS_OK'
