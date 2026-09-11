param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'BOUNDARY.ps1'), [Text.Encoding]::UTF8)))
function Get-Process {param($Id,$Name,$ErrorAction) if($Id -eq 42){[pscustomobject]@{Id=42}}}
try{Invoke-CdrMaintenanceLaunch $s $StatePath;throw 'live old accepted'}catch{if($_.Exception.Message -notmatch 'original_PID_still_present'){throw}}
if($script:starts -ne 0){throw 'started before old exit'}
