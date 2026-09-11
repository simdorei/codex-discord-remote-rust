Import-SourceFunctions 'codex-discord-rust-restart.ps1' | ForEach-Object {Invoke-Expression $_}
$BinaryPath=Join-Path $RepoRoot 'probe.cmd'
$Watchdog=Join-Path $RepoRoot 'watchdog.ps1'
[IO.File]::WriteAllText($BinaryPath,"@echo off`necho thread not loaded 1>&2`nexit /b 41`n")
[IO.File]::WriteAllText($Watchdog,"[IO.File]::WriteAllText((Join-Path `$env:V2_ROOT 'sealed'),'yes')`nexit 0`n")
$WaitTimeoutSeconds=0; $EffectiveQuietSeconds=15
function Get-VerifiedRustIdentity {'42|99'}
try {Invoke-RestartReadinessCheck -Identity '42|99'; throw 'preflight failure ignored'}
catch {if($_.Exception.Message -notmatch 'preflight' -or $_.Exception.Message -notmatch '41'){throw}}
if(Test-Path (Join-Path $RepoRoot 'sealed')) {throw 'intake sealed despite failed preflight'}
