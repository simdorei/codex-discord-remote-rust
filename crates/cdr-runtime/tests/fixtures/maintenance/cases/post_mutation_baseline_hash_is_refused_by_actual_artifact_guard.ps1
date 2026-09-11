param([string]$Variant)
switch ($Variant) {
'0' {
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceActions.ps1')
function Assert-CdrMaintenanceProgramPins {}
$EnvPath=Join-Path $RepoRoot '.env'
[IO.File]::WriteAllText($EnvPath,'fixture environment only')
$s=Read-CdrMaintenanceState $StatePath
[IO.File]::WriteAllText($BinaryPath,'baseline')
[IO.File]::WriteAllText($s.CandidatePath,'candidate')
[IO.File]::WriteAllText($s.OperatorPath,'operator')
$s.EnvHash=Get-CdrArtifactHash $EnvPath;$s.BaselineHash=Get-CdrArtifactHash $BinaryPath
$s.CandidateHash=Get-CdrArtifactHash $s.CandidatePath;$s.OperatorHash=Get-CdrArtifactHash $s.OperatorPath
$s.Phase='mutation_started'
try{Assert-CdrMaintenanceArtifacts $s;throw 'baseline accepted after mutation'}
catch{if($_.Exception.Message -notmatch 'installed_hash_wrong_for_phase'){throw}}
}
default { throw "Unknown native fixture variant: $Variant" }
}
