param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'RECONCILE.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$owner=Enter-CdrControl $RepoRoot -MaintenanceV2
try{
 try{. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply;throw 'lock bypassed'}
 catch{if($_.Exception.Message -notmatch 'cdr_control_busy'){throw}}
}finally{$owner.Dispose()}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'competing owner affected'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
