param([string]$Variant)
[IO.File]::Delete($StopPath);[IO.File]::Delete($DisablePath)
[IO.File]::WriteAllText((Join-Path $RepoRoot '.codex_discord_rust.maintenance.v2'),'owned state')
try{Invoke-CdrDeploymentRecovery $statePath;throw 'pending v2 ignored'}
catch{if($_.Exception.Message -notmatch 'maintenance_v2'){throw}}
if($script:starts -ne 0 -or $script:waits -ne 0){throw 'legacy action'}
