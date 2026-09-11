param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'BOUNDARY.ps1'), [Text.Encoding]::UTF8)))
$j=New-CdrLaunchJournal (Join-Path $s.Bundle 'launch.json') ('maintenance:'+$s.Operation) $s.Fence $s.CandidateHash
$j.Phase='launching';Save-CdrLaunchJournal (Join-Path $s.Bundle 'launch.json') $j
try{Invoke-CdrMaintenanceLaunch $s $StatePath;throw 'unknown accepted'}catch{if($_.Exception.Message -notmatch 'launch_outcome_unknown'){throw}}
if($script:starts -ne 0){throw 'unknown relaunched'}
