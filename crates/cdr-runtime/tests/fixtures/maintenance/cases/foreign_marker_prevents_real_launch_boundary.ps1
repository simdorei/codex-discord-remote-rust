param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'BOUNDARY.ps1'), [Text.Encoding]::UTF8)))
[IO.File]::WriteAllText($StopPath,'foreign')
try{Invoke-CdrMaintenanceLaunch $s $StatePath;throw 'foreign accepted'}catch{if($_.Exception.Message -notmatch 'Foreign maintenance marker'){throw}}
if($script:starts -ne 0 -or [IO.File]::ReadAllText($StopPath) -cne 'foreign'){throw 'foreign affected'}
