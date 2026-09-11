param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'RECONCILE.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$guard=[IO.File]::Open($StatePath,'Open','Read','Read')
try{try{. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply;throw 'fault absent'}catch{if($_.Exception.Message -eq 'fault absent'){throw}}}
finally{$guard.Dispose()}
if((Test-Path $DisablePath) -or -not (Test-Path $StatePath)){throw 'wrong partial boundary'}
if([Convert]::ToBase64String([IO.File]::ReadAllBytes($StatePath)) -cne $originalBytes){throw 'original state hash invalidated'}
. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply
if(Test-Path $StatePath){throw 'partial cleanup stuck'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
