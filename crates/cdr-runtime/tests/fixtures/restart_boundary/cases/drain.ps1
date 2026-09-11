param([string]$Variant)
. (Join-Path $env:V2_SOURCE 'codex-discord-rust-drain.ps1')
$DrainIdentityPath=Join-Path $RepoRoot 'identity'
$DrainPreparePath=Join-Path $RepoRoot 'prepare'
$DrainAckPath=Join-Path $RepoRoot 'ack'
function Get-RuntimeDrainIdentity {[pscustomobject]@{RuntimeId='runtime-a';ProcessIdentity='42|99'}}
function Get-VerifiedRuntimeIdentity {'42|99'}
function Get-VerifiedRuntimeProcess {[pscustomobject]@{Id=42}}
switch($Variant) {
 'timeout' {
  try {Enter-RestartDrain -Process ([pscustomobject]@{Id=42}) -ExpectedProcessIdentity '42|99' -TimeoutSeconds 0; throw 'expected timeout'}
  catch {if($_.Exception.Message -notmatch 'runtime_remains_sealed=true'){throw}}
  if(-not (Test-Path -LiteralPath $DrainPreparePath)){throw 'prepare silently removed'}
 }
 'stale_ack' {
  [IO.File]::WriteAllText($DrainPreparePath,"version=1`nruntime_id=runtime-a`nprocess_identity=42|99`nnonce=current`n")
  [IO.File]::WriteAllText($DrainAckPath,"version=1`nruntime_id=runtime-a`nprocess_identity=42|99`nnonce=stale`nstate=sealed`n")
  try {Enter-RestartDrain -Process ([pscustomobject]@{Id=42}) -ExpectedProcessIdentity '42|99' -TimeoutSeconds 1; throw 'stale ack accepted'}
  catch {if($_.Exception.Message -notmatch 'does not match'){throw}}
 }
 'orphan' {
  $paths=@($DrainIdentityPath,$DrainPreparePath,$DrainAckPath)
  foreach($p in $paths){[IO.File]::WriteAllText($p,'sentinel')}
  try {Clear-OrphanedRestartDrainArtifacts; throw 'cleanup accepted'}
  catch {if($_.Exception.Message -notmatch 'Refusing'){throw}}
  foreach($p in $paths){if([IO.File]::ReadAllText($p) -ne 'sentinel'){throw 'live markers changed'}}
  function Get-VerifiedRuntimeProcess {$null}
  Clear-OrphanedRestartDrainArtifacts
  foreach($p in $paths){if(Test-Path $p){throw 'orphan marker remains'}}
 }
 default {throw 'unknown case'}
}
