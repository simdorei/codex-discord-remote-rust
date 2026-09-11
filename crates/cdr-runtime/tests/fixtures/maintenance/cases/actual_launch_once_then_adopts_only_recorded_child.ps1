param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'BOUNDARY.ps1'), [Text.Encoding]::UTF8)))
Invoke-CdrMaintenanceLaunch $s $StatePath
Invoke-CdrMaintenanceLaunch $s $StatePath
if($script:starts -ne 1){throw 'duplicate launch'}
