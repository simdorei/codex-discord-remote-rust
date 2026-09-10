"""Real maintenance validators, completion and HTTP adapter in a synthetic root."""
from test_maintenance_v2_boundaries import BOUNDARY

CONNECTED = BOUNDARY + r'''
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceBackup.ps1')
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceActions.ps1')
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceNotification.ps1')
$EnvPath=Join-Path $RepoRoot '.env'
[IO.File]::WriteAllText($EnvPath,'DISCORD_BOT_TOKEN=synthetic-fixture-only')
$env:DISCORD_BOT_TOKEN='synthetic-fixture-only'
[IO.File]::WriteAllText($BinaryPath,'MZcandidate')
[IO.File]::WriteAllText($s.CandidatePath,'MZcandidate')
[IO.File]::WriteAllText($s.OperatorPath,'MZoperator')
[IO.File]::WriteAllText((Join-Path $s.Bundle 'baseline.exe'),'MZbaseline')
$s.CandidateHash=Get-CdrArtifactHash $BinaryPath
$s.OperatorHash=Get-CdrArtifactHash $s.OperatorPath
$s.BaselineHash=Get-CdrArtifactHash (Join-Path $s.Bundle 'baseline.exe')
$s.EnvHash=Get-CdrArtifactHash $EnvPath
$pins=@(Get-CdrMaintenanceProgramPaths | ForEach-Object {
 $p=Join-Path $RepoRoot $_
 [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($p))
 [IO.File]::Copy((Join-Path $env:V2_SOURCE $_),$p,$false)
 [pscustomobject]@{Path=$_;Hash=(Get-CdrArtifactHash $p)}
})
$s|Add-Member ProgramPins $pins
foreach($stage in @('PreStopBackup','PostStopBackup')) {
 $suffix=if($stage -eq 'PreStopBackup'){'aaaaaaaaaaaa'}else{'bbbbbbbbbbbb'}
 $snapshot=Join-Path $RepoRoot ('.codex-discord-backups/discord_mirror.v3-cutover.20260910T140000Z.'+$suffix+'.sqlite')
 [IO.File]::WriteAllText($snapshot,'synthetic backup proof')
 $s.$stage=[pscustomobject]@{
  Operation=$op;Policy=$s.ShutdownPolicy;Stage=$stage;BaselineHash=$s.BaselineHash;CandidateHash=$s.CandidateHash
  SourceDb=(Join-Path $RepoRoot 'discord_mirror.sqlite');Verified=$true;PackageVerified=$true
  SnapshotPath=$snapshot;SnapshotHash=(Get-CdrArtifactHash $snapshot)
 }
}
$s|Add-Member CompletionPolicy 'runtime-proof-v1'
$s|Add-Member PreviousCompletedHash ''
Save-CdrMaintenanceState $s $StatePath
# External process creation/readiness only; real launch journal/guards stay connected.
function Invoke-CdrMaintenanceFullReadiness {Effect 'full_readiness'}
Invoke-CdrMaintenanceLaunch $s $StatePath
$s.Phase='healthy';Save-CdrMaintenanceState $s $StatePath
function Get-HeartbeatHealth {[pscustomobject]@{Healthy=$true;Bootstrap=$false}}
$stamp=[DateTimeOffset]::UtcNow.ToUnixTimeSeconds()-1
[IO.File]::WriteAllText($HeartbeatPath,"pid=77`nupdated_at=$stamp`n")
function Start-Sleep {
 $stamp=[DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
 [IO.File]::WriteAllText($HeartbeatPath,"pid=77`nupdated_at=$stamp`n")
}
$script:posts=0
function Invoke-RestMethod {
 $script:posts++
 $ex=[Exception]::new('synthetic secret must never persist: bearer fixture-secret')
 $ex|Add-Member Response ([pscustomobject]@{StatusCode=403})
 $record=[Management.Automation.ErrorRecord]::new($ex,'fixture_http_403','PermissionDenied',$null)
 $record.ErrorDetails=[Management.Automation.ErrorDetails]::new('{"code":50013,"message":"Missing Permissions","extra":"fixture-secret"}')
 throw $record
}
'''
