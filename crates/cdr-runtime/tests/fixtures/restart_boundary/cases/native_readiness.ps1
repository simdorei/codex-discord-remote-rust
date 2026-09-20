Import-SourceFunctions 'codex-discord-rust-watchdog.ps1' | ForEach-Object {Invoke-Expression $_}
$BinaryPath=Join-Path $RepoRoot 'fake-rust-runtime.cmd'
$EnvPath=Join-Path $RepoRoot '.env';$RestartQuietSeconds=17;$RestartWaitTimeoutSeconds=23
$env:CODEX_DISCORD_PYTHON=Join-Path $RepoRoot 'fake-python.cmd'
[IO.File]::WriteAllText($env:CODEX_DISCORD_PYTHON,"@echo invoked>python-invoked.txt`n@exit /b 91`n")
[IO.File]::WriteAllText($BinaryPath,"@echo %*>rust-invoked.txt`n@cd>rust-cwd.txt`n@exit /b 0`n")
[IO.File]::WriteAllText((Join-Path $RepoRoot 'codex-discord-watchdog-restart-runtime.ps1'),@'
function Wait-CodexThreadsQuietForRestart {
 & $env:CODEX_DISCORD_PYTHON $BridgePath
 if($LASTEXITCODE -ne 0){throw 'fake Python failure'}
}
'@)
Wait-RustThreadsQuietForRestart
if(Test-Path (Join-Path $RepoRoot 'python-invoked.txt')){throw 'legacy interpreter invoked'}
$record=[IO.File]::ReadAllText((Join-Path $RepoRoot 'rust-invoked.txt')).Trim()
# The cmd fixture records raw argv text, including quotes around paths with spaces.
$expectedEnvPath=if($EnvPath -match '\s'){'"'+$EnvPath+'"'}else{$EnvPath}
$expected="--restart-readiness --restart-quiet-seconds 17 --restart-wait-timeout-seconds 23 --env $expectedEnvPath"
if($record.Replace('\\','\') -ne $expected){throw "wrong readiness arguments: $record"}
if([IO.Path]::GetFullPath([IO.File]::ReadAllText((Join-Path $RepoRoot 'rust-cwd.txt')).Trim()) -ne $RepoRoot){throw 'wrong working directory'}
