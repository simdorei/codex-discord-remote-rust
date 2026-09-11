param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'BOUNDARY.ps1'), [Text.Encoding]::UTF8)))
Invoke-CdrMaintenanceLaunch $s $StatePath
$s.Phase='verified';$s.Heartbeats=@(1,2);$s.DiscordReceipt='12345';Save-CdrMaintenanceState $s $StatePath
function Get-HeartbeatHealth {[pscustomobject]@{Healthy=$false;Bootstrap=$false}}
try{Complete-CdrMaintenance $s $StatePath;throw 'stale completed'}catch{if($_.Exception.Message -notmatch 'heartbeat_not_fresh'){throw}}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'stale seal released'}
