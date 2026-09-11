. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceBackup.ps1')
$s=Read-CdrMaintenanceState $StatePath
$EnvPath=Join-Path $RepoRoot '.env'
[IO.File]::WriteAllText($BinaryPath,'MZbaseline')
[IO.File]::WriteAllText($s.CandidatePath,'MZcandidate')
[IO.File]::WriteAllText($s.OperatorPath,'MZoperator')
$s.BaselineHash=Get-CdrArtifactHash $BinaryPath
$s.CandidateHash=Get-CdrArtifactHash $s.CandidatePath
Save-CdrMaintenanceState $s $StatePath
$snapshotDir=Join-Path $RepoRoot '.codex-discord-backups'
# LOAD's Directory.CreateDirectory(bundle) recursively creates this parent too.
if(-not [IO.Directory]::Exists($snapshotDir)){throw 'fixture parent missing after recursive bundle initialization'}
$snapshot=Join-Path $snapshotDir 'discord_mirror.v3-cutover.20260909T000000Z.aaaaaaaaaaaa.sqlite'
$script:nativeFailure=''
function Invoke-CdrMaintenanceCommand {
 param($State,$File,$Arguments,$LimitSeconds,[switch]$PassThru)
 if($Arguments[0] -eq $script:nativeFailure){throw 'injected_native_failure'}
 if($Arguments[0] -eq '--backup-store'){
  [IO.File]::WriteAllText($snapshot,'synthetic snapshot bytes; SQLite verified in separate integration test')
  return [pscustomobject]@{ExitCode=0;Stdout="backup_created path=$snapshot";Stderr=''}
 }
 return [pscustomobject]@{ExitCode=0;Stdout='config_valid token=[REDACTED]';Stderr=''}
}
