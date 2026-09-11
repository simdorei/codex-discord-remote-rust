param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PREPARED.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$latest=$StatePath+'.completed'
[IO.File]::WriteAllText($latest,'{"Operation":"older-ticket"}')
$s.PreviousCompletedHash=Get-CdrArtifactHash $latest;Save-CdrMaintenanceState $s $StatePath
$guard=[IO.File]::Open($latest,'Open','Read','Read')
try {try{Complete-CdrMaintenance $s $StatePath;throw 'failure missing'}catch{if($_.Exception.Message -eq 'failure missing'){throw}}}
finally{$guard.Dispose()}
if(-not (Test-Path $DisablePath) -or -not (Test-Path $StatePath)){throw 'failed receipt released ownership'}
Invoke-CdrMaintenanceEngine $StatePath $op
if((Test-Path $DisablePath) -or (Test-Path $StatePath)){throw 'owned cleanup did not resume'}
if($script:posts -ne 0 -or $script:starts -ne 1){throw 'reentry replayed side effect'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
