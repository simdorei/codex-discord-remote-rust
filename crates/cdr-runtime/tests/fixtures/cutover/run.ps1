param([string]$CaseFile, [string]$CaseName)
$ErrorActionPreference='Stop'
$RepoRoot=$env:PUBLISH_ROOT
. (Join-Path $env:PUBLISH_SOURCE 'codex-discord-rust-control.ps1')
$source=(Get-Content (Join-Path $env:PUBLISH_SOURCE 'codex-discord-runtime-cutover.ps1') -Raw).Replace("`r`n","`n")
foreach ($part in @('State','Runtime','Completion','Recovery')) {
    . (Join-Path $env:PUBLISH_SOURCE "scripts/CdrCutover$part.ps1")
}
$DisablePath=Join-Path $RepoRoot '.codex_discord_bot.disabled'
$RustLock=Join-Path $RepoRoot '.codex_discord_rust.runtime.lock'
$RustStop=Join-Path $RepoRoot '.codex_discord_rust.stop'
$RustRestart=Join-Path $RepoRoot '.codex_discord_rust.restart'
& $CaseFile -CaseName $CaseName
exit 0
