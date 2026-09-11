param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'RECONCILE.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
[IO.File]::AppendAllText($StatePath,' ')
try{. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply;throw 'invalid accepted'}
catch{if($_.Exception.Message -notmatch 'active_state_changed'){throw}}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'invalid cleanup performed'}
}
'1' {
[IO.File]::AppendAllText((Join-Path $RepoRoot 'scripts/CdrMaintenanceState.ps1'),'#changed')
try{. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply;throw 'invalid accepted'}
catch{if($_.Exception.Message -notmatch 'reviewed_entry_source_changed'){throw}}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'invalid cleanup performed'}
}
'2' {
[IO.File]::WriteAllText($BinaryPath,'changed')
try{. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply;throw 'invalid accepted'}
catch{if($_.Exception.Message -notmatch 'runtime_artifact_changed'){throw}}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'invalid cleanup performed'}
}
'3' {
$script:childStarted=$script:childStarted.AddSeconds(5)
try{. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply;throw 'invalid accepted'}
catch{if($_.Exception.Message -notmatch 'child_identity_changed'){throw}}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'invalid cleanup performed'}
}
'4' {
$script:childAlive=$false
try{. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply;throw 'invalid accepted'}
catch{if($_.Exception.Message -notmatch 'requires_one_runtime'){throw}}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'invalid cleanup performed'}
}
'5' {
[IO.File]::WriteAllText($StopPath,$op)
try{. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply;throw 'invalid accepted'}
catch{if($_.Exception.Message -notmatch 'new_or_foreign_intent'){throw}}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'invalid cleanup performed'}
}
'6' {
[IO.File]::WriteAllText($DisablePath,'another-operation')
try{. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply;throw 'invalid accepted'}
catch{if($_.Exception.Message -notmatch 'Foreign maintenance marker'){throw}}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'invalid cleanup performed'}
}
'7' {
[IO.File]::WriteAllText(($StatePath+'.completed'),'{"Operation":"newer-ticket"}')
try{. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply;throw 'invalid accepted'}
catch{if($_.Exception.Message -notmatch 'newer_completion_preserved'){throw}}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'invalid cleanup performed'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
