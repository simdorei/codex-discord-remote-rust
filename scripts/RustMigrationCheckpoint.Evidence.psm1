Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.Common.psm1') -ErrorAction Stop
Import-Module (Join-Path $PSScriptRoot 'CodexDiscordSoak.SourceFingerprint.psm1') -ErrorAction Stop
Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.EvidenceContract.psm1') -ErrorAction Stop
Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.SourceBinding.psm1') -ErrorAction Stop
$ErrorActionPreference = 'Stop'

function Read-CdrCheckpointJsonSnapshot {
    param([string]$Path, [string]$Label)
    $strictUtf8 = [Text.UTF8Encoding]::new($false, $true)
    try {
        $bytes = [IO.File]::ReadAllBytes($Path)
        $text = $strictUtf8.GetString($bytes)
        $trimmed = $text.Trim()
        if (-not $trimmed.StartsWith('{', [StringComparison]::Ordinal) -or
            -not $trimmed.EndsWith('}', [StringComparison]::Ordinal)) {
            throw "$Label root must be a JSON object"
        }
        $value = $text | ConvertFrom-Json
        $sha = [Security.Cryptography.SHA256]::Create()
        try {
            $hash = ([BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-', '')
        } finally { $sha.Dispose() }
        return [pscustomobject]@{
            Path = $Path; Value = $value; Hash = $hash; Bytes = [long]$bytes.Length
        }
    }
    catch { throw "$Label is not valid strict UTF-8 JSON: $($_.Exception.Message)" }
}

function Get-CdrCheckpointEvidenceBundle {
    [CmdletBinding()]
    param(
        [string]$RepoRoot,
        [string]$RepositoryEvidenceRoot,
        [string]$OfflineSoakEvidencePath,
        [string]$WorkspaceGateEvidencePath,
        [string]$ExpectedRuntimeSha256
    )
    $soakPath = Resolve-CdrCheckpointPath $RepoRoot $OfflineSoakEvidencePath
    $soakPath = Assert-CdrPathUnderRoot `
        $soakPath $RepositoryEvidenceRoot 'OfflineSoakEvidencePath'
    Assert-CdrCheckpointSource $soakPath $RepoRoot
    $gatePath = Resolve-CdrCheckpointPath $RepoRoot $WorkspaceGateEvidencePath
    $gatePath = Assert-CdrPathUnderRoot `
        $gatePath $RepositoryEvidenceRoot 'WorkspaceGateEvidencePath'
    Assert-CdrCheckpointSource $gatePath $RepoRoot

    $soakSnapshot = Read-CdrCheckpointJsonSnapshot $soakPath 'Offline soak evidence'
    $soak = $soakSnapshot.Value
    Assert-CdrOfflineSoakEvidenceContract $soak
    $gateSnapshot = Read-CdrCheckpointJsonSnapshot $gatePath 'Workspace gate evidence'
    $gate = $gateSnapshot.Value
    Assert-CdrWorkspaceGateEvidenceContract $gate

    $source = Get-CodexSoakSourceFingerprint -RepoRoot $RepoRoot
    if ($soak.provenance.source_fingerprint -cne $source.aggregate_sha256) {
        throw 'Offline soak evidence source fingerprint does not match the current Rust source.'
    }
    if ($soak.provenance.source_file_count -ne $source.file_count -or
        $soak.provenance.source_total_bytes -ne $source.total_bytes) {
        throw 'Offline soak evidence source dimensions do not match the current Rust source.'
    }
    if ($soak.artifacts.cdr_runtime.sha256 -cne $ExpectedRuntimeSha256) {
        throw 'Offline soak evidence cdr-runtime.exe SHA-256 mismatch.'
    }
    if ($gate.source_fingerprint -cne $source.aggregate_sha256) {
        throw 'Workspace gate evidence source fingerprint does not match the current Rust source.'
    }
    if ($gate.source_scope.schema -cne $source.schema -or
        $gate.source_scope.file_count -ne $source.file_count -or
        $gate.source_scope.total_bytes -ne $source.total_bytes) {
        throw 'Workspace gate evidence source dimensions do not match the current Rust source.'
    }
    $powerShellSource = Assert-CdrCheckpointPowerShellSourceRecord `
        -Record $gate.powershell_source -RepoRoot $RepoRoot
    $rollbackSource = Assert-CdrCheckpointRollbackSourceRecord `
        -Record $gate.rollback_source -RepoRoot $RepoRoot
    foreach ($name in @('cdr_runtime', 'cdr_offline_soak', 'cdr_mcp_server')) {
        if ($soak.artifacts.$name.sha256 -cne $gate.artifacts.$name.sha256 -or
            $soak.artifacts.$name.bytes -ne $gate.artifacts.$name.bytes) {
            throw "Offline soak and workspace gate artifact evidence disagree: $name"
        }
    }
    return [pscustomobject]@{
        SourceFingerprint = $source
        PowerShellSource = $powerShellSource
        RollbackSource = $rollbackSource
        OfflineSoak = $soak
        OfflineSoakPath = $soakPath
        OfflineSoakHash = $soakSnapshot.Hash
        OfflineSoakSnapshot = $soakSnapshot
        WorkspaceGate = $gate
        WorkspaceGatePath = $gatePath
        WorkspaceGateHash = $gateSnapshot.Hash
        WorkspaceGateSnapshot = $gateSnapshot
    }
}

Export-ModuleMember -Function 'Get-CdrCheckpointEvidenceBundle'
