param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'BOUNDARY.ps1'), [Text.Encoding]::UTF8)))
Invoke-CdrMaintenanceLaunch $s $StatePath
$script:sleeps=0
function Get-HeartbeatHealth {[pscustomobject]@{Healthy=$true;Bootstrap=$false}}
[IO.File]::WriteAllText($HeartbeatPath,"pid=77`nupdated_at=123`n")
function Start-Sleep {$script:sleeps++;if($script:sleeps -gt 2){throw 'fixture_clock_end'}}
try{Wait-CdrMaintenanceHeartbeats $s $StatePath;throw 'same heartbeat accepted'}catch{if($_.Exception.Message -notmatch 'fixture_clock_end'){throw}}
if((Read-CdrMaintenanceState $StatePath).Heartbeats.Count){throw 'false heartbeat receipt'}
