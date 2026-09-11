param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'BOUNDARY.ps1'), [Text.Encoding]::UTF8)))
function Invoke-CdrMaintenanceFullReadiness {param($State);$State.Deadline=[DateTimeOffset]::UtcNow.AddSeconds(-1).ToString('o')}
try{Invoke-CdrMaintenanceLaunch $s $StatePath;throw 'launch after deadline'}catch{if($_.Exception.Message -notmatch 'deadline'){throw}}
if($script:starts -ne 0){throw 'expired launch called'}
