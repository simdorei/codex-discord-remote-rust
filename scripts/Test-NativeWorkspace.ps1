[CmdletBinding()]
param([string]$RepoRoot, [switch]$SkipUnitTests)
$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($RepoRoot)) { $RepoRoot = Join-Path $PSScriptRoot '..' }
$RepoRoot = [IO.Path]::GetFullPath($RepoRoot)

function Invoke-Checked([scriptblock]$Command) {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { & $Command; $code = $LASTEXITCODE }
    finally { $ErrorActionPreference = $previous }
    if ($code -ne 0) { throw "Native QA command failed with exit code $code; see original output above." }
}

Push-Location $RepoRoot
try {
    $null = Get-Command cargo, node, git -ErrorAction Stop
    Invoke-Checked { git diff --check }
    & (Join-Path $RepoRoot 'scripts/Test-RustFormatting.ps1') -RepoRoot $RepoRoot
    Invoke-Checked { cargo build --workspace --locked }
    if (-not $SkipUnitTests) {
        Invoke-Checked { cargo test --workspace --locked --all-targets -- --test-threads=2 }
    }
    Invoke-Checked { cargo clippy --workspace --all-targets --locked -- -D warnings }
    Invoke-Checked { powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\install.ps1 -DryRun -SkipBuild -SkipEnvFile -SkipCodexPlugin }
    Invoke-Checked { powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\setup-discord-bot.ps1 -DryRun }

    $gitBash = $env:OMO_CODEX_GIT_BASH_PATH
    if ([string]::IsNullOrWhiteSpace($gitBash)) {
        $gitDirectory = Split-Path -Parent (Get-Command git -ErrorAction Stop).Source
        $candidates = @((Join-Path $gitDirectory 'bash.exe'), (Join-Path (Split-Path -Parent $gitDirectory) 'bin/bash.exe'))
        $gitBash = $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    }
    if ([string]::IsNullOrWhiteSpace($gitBash) -or -not (Test-Path -LiteralPath $gitBash -PathType Leaf)) {
        throw 'Git Bash is required to verify all supported shell wrappers; set OMO_CODEX_GIT_BASH_PATH.'
    }
    Invoke-Checked { & $gitBash -c 'set -e; bash -n install.sh setup-discord-bot.sh codex-discord-bot.sh; bash install.sh --dry-run --skip-build --skip-env-file --skip-codex-plugin; bash setup-discord-bot.sh --dry-run' }
    if ($SkipUnitTests) {
        Write-Output 'partial_checks_only: unit/integration tests were explicitly skipped; not deployment evidence.'
    } else {
        Write-Output 'native_workspace_checks_passed: no live Discord, deployment, or interpreter-unavailable proof is implied.'
    }
} finally { Pop-Location }
