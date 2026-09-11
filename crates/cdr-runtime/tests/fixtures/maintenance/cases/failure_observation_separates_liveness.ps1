param([string]$Variant)
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceFailure.ps1')
$s=Read-CdrMaintenanceState $StatePath
$s.LastError='graceful_exit_timeout private-details-not-for-discord'
$DrainAckPath=Join-Path $RepoRoot 'ack';$StopPath=Join-Path $RepoRoot 'stop'
function Get-Process {param($Id,$ErrorAction)
 if($Variant -eq 'unknown'){throw 'observation denied'}
 if($Variant -eq 'alive'){[pscustomobject]@{Id=42}}
}
function Get-RustProcessIdentity {'42|99'}
$observation=Get-CdrMaintenanceFailureObservation $s
if($observation.Original -cne $Variant){throw 'wrong liveness observation'}
if($observation.Code -cne 'graceful_exit_timeout' -or ($observation|ConvertTo-Json) -match 'private-details'){throw 'unsafe error code'}
if(-not $observation.ObservedAt -or $observation.Ack -cne 'missing' -or $observation.Stop -cne 'missing'){throw 'observation incomplete'}
