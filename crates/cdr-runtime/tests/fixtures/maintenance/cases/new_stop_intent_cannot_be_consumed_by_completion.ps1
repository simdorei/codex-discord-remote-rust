param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'BOUNDARY.ps1'), [Text.Encoding]::UTF8)))
Invoke-CdrMaintenanceLaunch $s $StatePath
$s.Phase='verified';$s.Heartbeats=@(1,2);$s.DiscordReceipt='12345';Save-CdrMaintenanceState $s $StatePath
[IO.File]::WriteAllText($StopPath,$s.Operation)
function Get-HeartbeatHealth {[pscustomobject]@{Healthy=$true;Bootstrap=$false}}
try{Complete-CdrMaintenance $s $StatePath;throw 'new stop consumed'}catch{if($_.Exception.Message -notmatch 'post_launch_intent_present'){throw}}
if(-not (Test-Path $StopPath) -or -not (Test-Path $DisablePath)){throw 'intent lost'}
