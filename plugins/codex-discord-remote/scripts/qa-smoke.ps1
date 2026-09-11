[CmdletBinding()]
param([string]$RepoRoot, [switch]$SkipUnitTests)
$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $RepoRoot = Join-Path $PSScriptRoot '..\..\..'
}
$RepoRoot = [IO.Path]::GetFullPath($RepoRoot)
& (Join-Path $RepoRoot 'scripts/Test-NativeWorkspace.ps1') -RepoRoot $RepoRoot -SkipUnitTests:$SkipUnitTests
