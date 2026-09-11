[CmdletBinding()]
param(
    [string]$RepoRoot
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $RepoRoot = Join-Path $PSScriptRoot '..\..\..'
}

$RepoRoot = [IO.Path]::GetFullPath($RepoRoot)
$RuntimeModePath = Join-Path $RepoRoot '.codex_discord_runtime'
$RuntimeMode = [string]$env:CODEX_DISCORD_RUNTIME
if ([string]::IsNullOrWhiteSpace($RuntimeMode) -and (Test-Path -LiteralPath $RuntimeModePath)) {
    $RuntimeMode = (Get-Content -LiteralPath $RuntimeModePath -Raw).Trim()
}
if ([string]::IsNullOrWhiteSpace($RuntimeMode)) {
    $RuntimeMode = 'rust'
}
if ($RuntimeMode -eq 'rust') {
    & (Join-Path $RepoRoot 'codex-discord-rust-status.ps1') -RepoRoot $RepoRoot
    exit $LASTEXITCODE
}
throw "This installation only supports the Rust runtime; unsupported selection: '$RuntimeMode'."
