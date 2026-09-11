param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'BOUNDARY.ps1'), [Text.Encoding]::UTF8)))
$j=New-CdrLaunchJournal (Join-Path $s.Bundle 'launch.json') ('maintenance:'+$s.Operation) $s.Fence $s.CandidateHash
$j.Phase='child';$j.ChildIdentity='77|99';Save-CdrLaunchJournal (Join-Path $s.Bundle 'launch.json') $j
try{Invoke-CdrMaintenanceLaunch $s $StatePath;throw 'dead child accepted'}catch{if($_.Exception.Message -notmatch 'recorded_child_dead'){throw}}
if($script:starts -ne 0 -or (Read-CdrLaunchJournal (Join-Path $s.Bundle 'launch.json')).Phase -ne 'child'){throw 'launch reset'}
