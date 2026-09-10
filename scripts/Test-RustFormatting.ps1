[CmdletBinding()]
param([string]$RepoRoot)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $RepoRoot = Join-Path $PSScriptRoot '..'
}
$RepoRoot = [IO.Path]::GetFullPath($RepoRoot)
Push-Location $RepoRoot
try {
    $metadataText = & cargo metadata --no-deps --format-version 1 --locked
    if ($LASTEXITCODE -ne 0) { throw 'Could not read Cargo workspace metadata.' }
    $metadata = $metadataText | ConvertFrom-Json
    $members = @($metadata.workspace_members)
    foreach ($package in $metadata.packages) {
        if ($package.id -notin $members) { continue }
        # One workspace-wide rustfmt command exceeds Windows' argument limit.
        # Keep the exact formatting check, but invoke it per workspace package.
        & cargo fmt --package $package.name -- --check
        if ($LASTEXITCODE -ne 0) {
            throw "Rust formatting check failed for $($package.name)."
        }
    }
} finally {
    Pop-Location
}
