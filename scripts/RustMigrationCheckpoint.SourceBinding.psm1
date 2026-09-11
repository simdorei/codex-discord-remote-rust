Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.Common.psm1') -ErrorAction Stop
Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.Payload.psm1') -ErrorAction Stop
Set-StrictMode -Version Latest

function Test-CdrCheckpointInteger {
    param([AllowNull()][object]$Value, [long]$Minimum)
    $integer = $Value -is [byte] -or $Value -is [uint16] -or
        $Value -is [uint32] -or $Value -is [uint64] -or
        $Value -is [sbyte] -or $Value -is [int16] -or
        $Value -is [int32] -or $Value -is [int64]
    return $integer -and [decimal]$Value -ge [decimal]$Minimum
}

function Get-CdrCheckpointRollbackSourceRecord {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)
    $records = [Collections.Generic.List[object]]::new()
    foreach ($entry in @(Get-CdrCheckpointRollbackSourceEntries -RepoRoot $RepoRoot)) {
        $item = Get-Item -LiteralPath $entry.source
        $records.Add([pscustomobject][ordered]@{
                path = $entry.source_path
                archive_path = $entry.archive_path
                sha256 = Get-CdrSha256 $entry.source
                bytes = [long]$item.Length
            })
    }
    return [pscustomobject][ordered]@{
        schema = 'cdr.checkpoint-rollback-source.v1'
        file_count = [int]$records.Count
        files = [object[]]$records.ToArray()
    }
}

function Get-CdrCheckpointPowerShellSourceRecord {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)
    $rollback = Get-CdrCheckpointRollbackSourceRecord -RepoRoot $RepoRoot
    $records = @($rollback.files | Where-Object {
            [IO.Path]::GetExtension([string]$_.path).ToLowerInvariant() -in @('.ps1', '.psm1')
        })
    return [pscustomobject][ordered]@{
        schema = 'cdr.checkpoint-powershell-source.v1'
        file_count = [int]$records.Count
        files = [object[]]@($records | ForEach-Object {
                [pscustomobject][ordered]@{
                    path = $_.path
                    sha256 = $_.sha256
                    bytes = $_.bytes
                }
            })
    }
}

function Assert-CdrCheckpointRollbackSourceRecord {
    param(
        [Parameter(Mandatory = $true)][AllowNull()][object]$Record,
        [Parameter(Mandatory = $true)][string]$RepoRoot
    )
    if ($null -eq $Record -or $Record -is [array] -or
        $Record.schema -isnot [string] -or
        $Record.schema -cne 'cdr.checkpoint-rollback-source.v1') {
        throw 'Workspace gate rollback source contract failed: schema is invalid'
    }
    if (-not (Test-CdrCheckpointInteger $Record.file_count 1) -or
        $Record.files -isnot [array]) {
        throw 'Workspace gate rollback source contract failed: file count or files type is invalid'
    }
    $rows = @($Record.files)
    if ([int64]$Record.file_count -ne $rows.Count) {
        throw 'Workspace gate rollback source contract failed: file count is invalid'
    }
    foreach ($actual in $rows) {
        if ($null -eq $actual -or $actual -is [array] -or
            $actual.path -isnot [string] -or
            $actual.archive_path -isnot [string] -or
            $actual.sha256 -isnot [string] -or
            $actual.sha256 -cnotmatch '^[A-F0-9]{64}$' -or
            -not (Test-CdrCheckpointInteger $actual.bytes 1)) {
            throw 'Workspace gate rollback source contract failed: file row type is invalid'
        }
    }
    $expected = Get-CdrCheckpointRollbackSourceRecord -RepoRoot $RepoRoot
    if ([int64]$Record.file_count -ne $expected.file_count) {
        throw 'Workspace gate rollback source contract failed: file count is invalid'
    }
    for ($index = 0; $index -lt $expected.files.Count; $index++) {
        $actual = $rows[$index]; $wanted = $expected.files[$index]
        if ($actual.path -cne $wanted.path -or
            $actual.archive_path -cne $wanted.archive_path) {
            throw "Rollback source path mismatch at index $index"
        }
        if ($actual.sha256 -cne $wanted.sha256) {
            throw "Rollback source hash mismatch: $($wanted.path)"
        }
        if ([int64]$actual.bytes -ne $wanted.bytes) {
            throw "Rollback source byte length mismatch: $($wanted.path)"
        }
    }
    return $expected
}

function Assert-CdrCheckpointPowerShellSourceRecord {
    param(
        [Parameter(Mandatory = $true)][AllowNull()][object]$Record,
        [Parameter(Mandatory = $true)][string]$RepoRoot
    )
    if ($null -eq $Record -or $Record -is [array] -or
        $Record.schema -isnot [string] -or
        $Record.schema -cne 'cdr.checkpoint-powershell-source.v1') {
        throw 'Workspace gate PowerShell source contract failed: schema is invalid'
    }
    if (-not (Test-CdrCheckpointInteger $Record.file_count 1) -or
        $Record.files -isnot [array]) {
        throw 'Workspace gate PowerShell source contract failed: file count or files type is invalid'
    }
    $rows = @($Record.files)
    if ([int64]$Record.file_count -ne $rows.Count) {
        throw 'Workspace gate PowerShell source contract failed: file count is invalid'
    }
    foreach ($actual in $rows) {
        if ($null -eq $actual -or $actual -is [array] -or
            $actual.path -isnot [string] -or
            $actual.sha256 -isnot [string] -or
            $actual.sha256 -cnotmatch '^[A-F0-9]{64}$' -or
            -not (Test-CdrCheckpointInteger $actual.bytes 1)) {
            throw 'Workspace gate PowerShell source contract failed: file row type is invalid'
        }
    }
    $expected = Get-CdrCheckpointPowerShellSourceRecord -RepoRoot $RepoRoot
    if ([int64]$Record.file_count -ne $expected.file_count) {
        throw 'Workspace gate PowerShell source contract failed: file count is invalid'
    }
    for ($index = 0; $index -lt $expected.files.Count; $index++) {
        $actual = $rows[$index]; $wanted = $expected.files[$index]
        if ($actual.path -cne $wanted.path) {
            throw "PowerShell source path mismatch at index $index"
        }
        if ($actual.sha256 -cne $wanted.sha256) {
            throw "PowerShell source hash mismatch: $($wanted.path)"
        }
        if ([int64]$actual.bytes -ne $wanted.bytes) {
            throw "PowerShell source byte length mismatch: $($wanted.path)"
        }
    }
    return $expected
}

function Get-CdrCheckpointRollbackRecordSha256([object]$Record) {
    $frame = [Text.StringBuilder]::new()
    $null = $frame.Append("cdr.observation-rollback.v1`0$($Record.file_count)`n")
    foreach ($file in $Record.files) {
        $null = $frame.Append("$($file.path)`0$($file.archive_path)`0$($file.sha256)`0$($file.bytes)`n")
    }
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [Text.UTF8Encoding]::new($false, $true).GetBytes($frame.ToString())
        return ([BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-', '')
    } finally { $sha.Dispose() }
}

Export-ModuleMember -Function @(
    'Get-CdrCheckpointRollbackRecordSha256',
    'Get-CdrCheckpointRollbackSourceRecord',
    'Assert-CdrCheckpointRollbackSourceRecord',
    'Get-CdrCheckpointPowerShellSourceRecord',
    'Assert-CdrCheckpointPowerShellSourceRecord'
)
