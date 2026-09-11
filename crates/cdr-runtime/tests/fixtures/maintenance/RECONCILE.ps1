. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'CONNECTED.ps1'), [Text.Encoding]::UTF8)))
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceReconcileManifest.ps1')
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceObservation.ps1')
$j=Read-CdrLaunchJournal (Join-Path $s.Bundle 'launch.json')
$nextBinary=Join-Path $RepoRoot 'target/release/cdr-runtime.exe'
[void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($nextBinary))
[IO.File]::Move($BinaryPath,$nextBinary);$BinaryPath=$nextBinary;$s.BinaryPath=$nextBinary;$j.BinaryPath=$nextBinary
$script:childStarted=[datetime]::UtcNow.AddMinutes(-5)
function Get-Process {param($Id,$Name,$ErrorAction)
 if($script:childAlive -and ($Name -eq 'cdr-runtime' -or $Id -eq 77)){
  [pscustomobject]@{Id=77;Path=$BinaryPath;StartTime=$script:childStarted}
 }
}
$j.ChildIdentity=Get-CdrCompletionProcessIdentity (Get-Process -Id 77)
Save-CdrLaunchJournal (Join-Path $s.Bundle 'launch.json') $j
$LockPath=Join-Path $RepoRoot '.codex_discord_rust.runtime.lock'
[IO.File]::WriteAllText($LockPath,"pid=77`n")
$s.PSObject.Properties.Remove('CompletionPolicy');$s.PSObject.Properties.Remove('PreviousCompletedHash')
$s.Phase='notifying';$s.Halted=$true;$s.LastError='maintenance_notification_outcome_unknown; no automatic resend'
$s.CreatedAt=$now.AddMinutes(-40).ToString('o');$s.Deadline=$now.AddMinutes(-10).ToString('o')
Save-CdrMaintenanceState $s $StatePath
$originalRoot=Join-Path $s.Bundle 'completion-original'
[void][IO.Directory]::CreateDirectory($originalRoot)
[IO.File]::Copy($StatePath,(Join-Path $originalRoot 'state.json'),$false)
foreach($pin in $s.ProgramPins){
 $target=Join-Path $originalRoot $pin.Path
 [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))
 [IO.File]::Copy((Join-Path $RepoRoot $pin.Path),$target,$false)
}
$sourcePins=@(Get-CdrReconcileSourcePaths|ForEach-Object {
 $target=Join-Path $RepoRoot $_
 if(-not [IO.File]::Exists($target)){
  [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))
  [IO.File]::Copy((Join-Path $env:V2_SOURCE $_),$target,$false)
 }
 [pscustomobject]@{Path=$_;Hash=(Get-CdrArtifactHash $target)}
})
$manifestPath=Join-Path $s.Bundle 'completion-review.json'
$manifest=[pscustomobject]@{Version=1;Kind='completion-only';Operation=$op;RepoRoot=$RepoRoot;
 OriginalStateHash=(Get-CdrArtifactHash $StatePath);CandidateHash=$s.CandidateHash;
 ChildIdentity=$j.ChildIdentity;PreviousCompletedHash='';SourcePins=$sourcePins}
Write-NewCdrMarker $manifestPath ($manifest|ConvertTo-Json -Depth 8 -Compress)
$manifestHash=Get-CdrArtifactHash $manifestPath
$entry=Join-Path $RepoRoot 'scripts/Complete-CdrMaintenance.ps1'
$originalBytes=[Convert]::ToBase64String([IO.File]::ReadAllBytes($StatePath))
function Invoke-RestMethod {throw 'RECOVERY MUST NOT POST'}
function Invoke-CdrMaintenanceEngine {throw 'RECOVERY MUST NOT RUN DEPLOYMENT ENGINE'}
function Start-Process {throw 'RECOVERY MUST NOT START PROCESSES'}
function Stop-Process {throw 'RECOVERY MUST NOT STOP PROCESSES'}
