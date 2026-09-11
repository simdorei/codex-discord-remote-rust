param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'RECONCILE.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$forged=$s|ConvertTo-Json -Depth 12|ConvertFrom-Json
$forged.Phase='verified';$forged|Add-Member CompletionPolicy 'runtime-proof-v1'
$forged|Add-Member RuntimeEvidence ([pscustomobject]@{ChildIdentity=$manifest.ChildIdentity})
$forged|Add-Member Reconciliation ([pscustomobject]@{ManifestHash=('F'*64)})
Write-NewCdrMarker (Join-Path $s.Bundle 'completion.json') ($forged|ConvertTo-Json -Depth 12)
try{. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply;throw 'FAULT_NOT_REJECTED'}
catch{if($_.Exception.Message -notmatch 'reconcile_receipt_authority_mismatch'){throw}}
if(-not (Test-Path $DisablePath) -or -not (Test-Path $StatePath)){throw 'wrong manifest proof unsealed'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
