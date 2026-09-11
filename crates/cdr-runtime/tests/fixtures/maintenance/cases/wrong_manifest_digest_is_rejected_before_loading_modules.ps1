param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'RECONCILE.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
try{. $entry -ManifestPath $manifestPath -ManifestHash ('A'*64) -ExpectedOperation $op -Apply;throw 'digest accepted'}
catch{if($_.Exception.Message -notmatch 'manifest_hash_mismatch'){throw}}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'digest mismatch mutated state'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
