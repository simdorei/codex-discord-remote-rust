Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.Common.psm1') -ErrorAction Stop
Set-StrictMode -Version Latest

function Copy-CdrCheckpointPayloadFile {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$StagingRoot,
        [Parameter(Mandatory = $true)][string]$ArchiveRelativePath,
        [string]$ExpectedSha256,
        [long]$ExpectedBytes = -1,
        [string]$ExpectedLabel = 'Checkpoint payload'
    )
    Assert-CdrCheckpointSource $Source $RepoRoot
    $relative = $ArchiveRelativePath.Replace('\', '/')
    Assert-ForbiddenArchiveName $relative
    $target = Join-Path $StagingRoot $relative.Replace('/', '\')
    $null = New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force
    Copy-Item -LiteralPath $Source -Destination $target
    Assert-CdrCheckpointLeaf $target 'Staged checkpoint payload'
    $item = Get-Item -LiteralPath $target
    $record = [pscustomobject][ordered]@{
        path = $relative
        sha256 = Get-CdrSha256 $target
        bytes = [long]$item.Length
    }
    if (-not [string]::IsNullOrWhiteSpace($ExpectedSha256) -and
        $record.sha256 -cne $ExpectedSha256) {
        throw "$ExpectedLabel SHA-256 mismatch after staging."
    }
    if ($ExpectedBytes -ge 0 -and $record.bytes -ne $ExpectedBytes) {
        throw "$ExpectedLabel byte length mismatch after staging."
    }
    return $record
}

Export-ModuleMember -Function 'Copy-CdrCheckpointPayloadFile'
