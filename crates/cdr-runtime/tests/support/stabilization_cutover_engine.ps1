# Engine fixture: OS effects are spies, but persisted phases use the real engine.
. (Join-Path $moduleRoot 'scripts/CdrMaintenanceEngine.ps1')
$State.Phase='planned'
$State | Add-Member CompletionPolicy 'runtime-proof-v1'
$State | Add-Member PreviousCompletedHash ''
$statePath=Get-CdrMaintenancePath $RepoRoot
Write-NewCdrMarker $statePath ($State | ConvertTo-Json -Depth 12)
$script:events=[Collections.Generic.List[string]]::new()
function Assert-CdrMaintenanceArtifacts {}
function Assert-CdrMaintenanceMarkers {}
function Assert-CdrMaintenanceRecoveryArmed {}
function Invoke-CdrMaintenancePreflight { $script:events.Add('preflight') }
function Invoke-CdrMaintenancePreStopBackup { $script:events.Add('backup') }
function Assert-CdrMaintenancePreStopBackup {}
function Assert-CdrMaintenanceBackupReceipt {}
function Invoke-CdrMaintenanceDrain { $script:events.Add('drain') }
function Assert-RestartDrainBound {}
function Invoke-CdrMaintenanceStop { $script:events.Add('stop') }
function Assert-CdrMaintenanceNoRuntime {}
function Invoke-CdrMaintenancePackaging { $script:events.Add('post-stop-backup') }
function Install-CdrMaintenanceCandidate { $script:events.Add('install') }
function Enable-CdrMaintenanceCompatibility {
    param($s)
    if ($s.Phase -cne 'candidate_installed') { throw 'arming phase was not durable' }
    $script:events.Add('arm')
    if ($Scenario -eq 'engine-failure') { throw 'fixture_arming_outcome_unknown' }
}
function Assert-WriterOrder {
    if (-not $script:events.Contains('arm')) { throw 'candidate_writer_before_compatibility_arming' }
    if ($script:events.IndexOf('arm') -lt $script:events.IndexOf('install')) { throw 'arming preceded install' }
}
function Invoke-CdrMaintenanceCleanup { Assert-WriterOrder; $script:events.Add('cleanup-no-disposal') }
function Invoke-CdrMaintenanceFullReadiness { Assert-WriterOrder; $script:events.Add('readiness') }
function Invoke-CdrMaintenanceLaunch { Assert-WriterOrder; $script:events.Add('launch') }
function Wait-CdrMaintenanceHeartbeats {}
function Get-CdrRuntimeCompletionEvidence { [pscustomobject]@{fixture=$true} }
function Complete-CdrMaintenance { $script:events.Add('complete') }
function Send-CdrCompletedNotice {}
function Get-CdrMaintenanceFailureObservation { [pscustomobject]@{fixture=$true} }
function Publish-CdrMaintenanceFailure {}
$errorText=''
try { Invoke-CdrMaintenanceEngine $statePath $State.Operation } catch { $errorText=$_.Exception.Message }
if ($Scenario -eq 'engine-order') {
    if ($errorText) { throw $errorText }
    if (@($script:events | Where-Object {$_ -ceq 'launch'}).Count -ne 1) { throw 'engine did not launch once' }
    if ($script:events.IndexOf('stop') -gt $script:events.IndexOf('arm')) { throw 'arming before stop' }
} else {
    if ($errorText -cne 'fixture_arming_outcome_unknown') { throw "wrong arming rejection: $errorText" }
    $persisted=Read-CdrMaintenanceState $statePath
    if (-not $persisted.Halted -or $persisted.Phase -cne 'candidate_installed') { throw 'unknown arming not retained' }
    foreach ($event in @('cleanup-no-disposal','readiness','launch','complete')) {
        if ($script:events.Contains($event)) { throw "effect after failed arming: $event" }
    }
}
Write-Output ($script:events -join ',')
