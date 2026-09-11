param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'RECONCILE.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($StatePath)) -cne $originalBytes){throw 'preflight mutated original'}
if(-not (Test-Path $DisablePath) -or (Test-Path ($StatePath+'.completed'))){throw 'preflight finalized'}
. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply
if((Test-Path $StatePath) -or (Test-Path $DisablePath)){throw 'explicit cleanup incomplete'}
$receipt=Get-Content ($StatePath+'.completed') -Raw|ConvertFrom-Json
if($receipt.Reconciliation.OriginalStateHash -cne $manifest.OriginalStateHash -or $receipt.Phase -cne 'verified'){throw 'new proof missing'}
if($receipt.LastError -cne $s.LastError -or -not $receipt.Halted){throw 'historical error erased'}
if((Read-CdrMaintenanceNotice $s).Status -cne 'unknown'){throw 'unknown notice changed'}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes((Join-Path $originalRoot 'state.json'))) -cne $originalBytes){throw 'historical snapshot changed'}
. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply
if($script:starts -ne 1 -or $script:posts -ne 0){throw 'reconciliation replayed side effect'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
