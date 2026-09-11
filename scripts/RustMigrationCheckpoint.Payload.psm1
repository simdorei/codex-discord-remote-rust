Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.Common.psm1') -ErrorAction Stop
Set-StrictMode -Version Latest

$script:CheckpointToolPaths = [string[]]@(
    'scripts/New-RustMigrationCheckpoint.ps1',
    'scripts/RustMigrationCheckpoint.Common.psm1',
    'scripts/RustMigrationCheckpoint.Package.psm1',
    'scripts/RustMigrationCheckpoint.Payload.psm1',
    'scripts/RustMigrationCheckpoint.Verify.psm1',
    'scripts/RustMigrationCheckpoint.RollbackVerify.psm1',
    'scripts/RustMigrationCheckpoint.Evidence.psm1',
    'scripts/RustMigrationCheckpoint.EvidenceContract.psm1',
    'scripts/RustMigrationCheckpoint.SourceBinding.psm1',
    'scripts/RustMigrationCheckpoint.Stage.psm1',
    'scripts/RustMigrationCheckpoint.ArchiveContract.psm1',
    'scripts/RustMigrationCheckpoint.ArchivePublish.psm1',
    'scripts/RustMigrationCheckpoint.EvidenceShape.psm1',
    'scripts/RustMigrationCheckpoint.NativeEvidence.psm1',
    'scripts/RustMigrationCheckpoint.Quality.psm1',
    'scripts/RustMigrationCheckpoint.ProcessSafety.psm1',
    'scripts/CodexDiscordSoak.Evidence.psm1',
    'scripts/CodexDiscordSoak.ProcessOwnership.psm1',
    'scripts/CodexDiscordSoak.Provenance.psm1',
    'scripts/CodexDiscordSoak.SourceFingerprint.psm1',
    'scripts/CodexDiscordSoak.WrapperEvidence.psm1',
    'scripts/CodexDiscordSoak.WrapperRuntime.psm1'
)

function Get-CdrCheckpointToolPaths { return [string[]]$script:CheckpointToolPaths.Clone() }

function Get-CdrCheckpointWorkspacePaths {
    return [string[]]@('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml')
}

function Get-CdrCheckpointRollbackSourceEntries {
    [CmdletBinding()]
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $root = [IO.Path]::GetFullPath($RepoRoot).TrimEnd('\', '/')
    $entries = [Collections.Generic.List[object]]::new()
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)

    function Add-SourceFile([string]$Path) {
        Assert-CdrCheckpointSource $Path $root
        $full = [IO.Path]::GetFullPath($Path)
        $relative = $full.Substring($root.Length).TrimStart('\', '/').Replace('\', '/')
        if (-not $seen.Add($relative)) { return }
        $entries.Add([pscustomobject][ordered]@{
                source = $full
                source_path = $relative
                archive_path = "operations/$relative"
            })
    }

    foreach ($item in @(Get-ChildItem -LiteralPath $root -File | Where-Object {
                $_.Extension -in @('.ps1', '.sh', '.cmd', '.vbs')
            } | Sort-Object Name)) { Add-SourceFile $item.FullName }
    foreach ($relative in $script:CheckpointToolPaths) {
        Add-SourceFile (Join-Path $root $relative.Replace('/', '\'))
    }
    # Reviewed policy is bound/package data, not an executable PowerShell tool.
    Add-SourceFile (Join-Path $root 'scripts/RustMigrationCheckpoint.QualityApprovals.json')

    $trees = @(
        [pscustomobject]@{
            path = 'scripts'
            extensions = @('.ps1', '.psm1')
        },
        [pscustomobject]@{
            path = 'plugins/codex-discord-remote'
            extensions = @('.json', '.md', '.mjs', '.ps1', '.yaml', '.yml')
        },
        [pscustomobject]@{
            path = '.agents/skills/ask-chatgpt-pro'
            extensions = @('.md', '.mjs', '.yaml', '.yml')
        }
    )
    foreach ($tree in $trees) {
        $directory = Join-Path $root $tree.path.Replace('/', '\')
        if (-not (Test-Path -LiteralPath $directory -PathType Container)) { continue }
        foreach ($item in @(Get-ChildItem -LiteralPath $directory -Recurse -File | Where-Object {
                    $_.FullName -notmatch '[\\/]__pycache__[\\/]' -and
                    $_.Extension.ToLowerInvariant() -in $tree.extensions
                } | Sort-Object FullName)) { Add-SourceFile $item.FullName }
    }
    $ordinal = [Collections.Generic.SortedDictionary[string, object]]::new(
        [StringComparer]::Ordinal
    )
    foreach ($entry in $entries) { $ordinal.Add([string]$entry.source_path, $entry) }
    return [object[]]@($ordinal.Values)
}

Export-ModuleMember -Function @(
    'Get-CdrCheckpointToolPaths',
    'Get-CdrCheckpointWorkspacePaths',
    'Get-CdrCheckpointRollbackSourceEntries'
)
