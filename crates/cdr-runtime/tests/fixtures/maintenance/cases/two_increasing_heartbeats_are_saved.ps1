param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'BOUNDARY.ps1'), [Text.Encoding]::UTF8)))
Invoke-CdrMaintenanceLaunch $s $StatePath
function Get-HeartbeatHealth {[pscustomobject]@{Healthy=$true;Bootstrap=$false}}
[IO.File]::WriteAllText($HeartbeatPath,"pid=77`nupdated_at=123`n")
function Start-Sleep {[IO.File]::WriteAllText($HeartbeatPath,"pid=77`nupdated_at=124`n")}
Wait-CdrMaintenanceHeartbeats $s $StatePath
if(((Read-CdrMaintenanceState $StatePath).Heartbeats -join ',') -cne '123,124'){throw 'heartbeat evidence missing'}
