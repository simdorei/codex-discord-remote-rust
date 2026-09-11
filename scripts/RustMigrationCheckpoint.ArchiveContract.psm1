Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.Common.psm1') -ErrorAction Stop
Set-StrictMode -Version Latest

function Test-CdrArchiveInteger([AllowNull()][object]$Value, [long]$Minimum) {
    $integer = $Value -is [byte] -or $Value -is [uint16] -or
        $Value -is [uint32] -or $Value -is [uint64] -or
        $Value -is [sbyte] -or $Value -is [int16] -or
        $Value -is [int32] -or $Value -is [int64]
    return $integer -and [decimal]$Value -ge [decimal]$Minimum
}

function Assert-CdrArchiveCanonicalPath([AllowNull()][object]$Value, [string]$Label) {
    if ($Value -isnot [string] -or [string]::IsNullOrWhiteSpace($Value) -or
        $Value.Contains('\') -or [IO.Path]::IsPathRooted($Value) -or
        $Value -match '(^|/)\.\.?(/|$)' -or $Value.Contains(':')) {
        throw "$Label is unsafe or non-canonical: $Value"
    }
    Assert-ForbiddenArchiveName $Value
}

function New-CdrArchiveRecordMap([AllowNull()][object]$Records, [string]$Label) {
    if ($Records -isnot [array] -or @($Records).Count -eq 0) {
        throw "$Label must be a non-empty array"
    }
    $map = [Collections.Generic.SortedDictionary[string, object]]::new(
        [StringComparer]::Ordinal
    )
    foreach ($record in @($Records)) {
        if ($null -eq $record -or $record -is [array] -or
            $record.path -isnot [string] -or
            $record.sha256 -isnot [string] -or
            $record.sha256 -cnotmatch '^[A-F0-9]{64}$' -or
            -not (Test-CdrArchiveInteger $record.bytes 1)) {
            throw "$Label contains an invalid record"
        }
        Assert-CdrArchiveCanonicalPath $record.path "$Label path"
        if ($map.ContainsKey($record.path)) { throw "$Label contains duplicate path: $($record.path)" }
        $map.Add([string]$record.path, $record)
    }
    return ,$map
}

function Assert-CdrCheckpointMetadataContract([object]$Metadata, [object]$Expected) {
    if ($null -eq $Metadata -or $Metadata -is [array] -or
        -not (Test-CdrArchiveInteger $Metadata.schema_version 1) -or
        [int64]$Metadata.schema_version -ne 1 -or
        $Metadata.kind -isnot [string] -or
        $Metadata.kind -cne 'cdr_rust_migration_local_release_checkpoint' -or
        $Metadata.checkpoint_stage -isnot [string] -or
        $Metadata.checkpoint_stage -cne 'final_source_local_checkpoint') {
        throw 'Checkpoint metadata identity contract failed.'
    }
    foreach ($name in @(
        'bot_disabled', 'local_checkpoint_only',
        'immutable_release_checkpoint_still_pending_authorization'
    )) {
        if ($Metadata.$name -isnot [bool] -or -not $Metadata.$name) {
            throw "Checkpoint metadata $name must be scalar true."
        }
    }
    if ($Metadata.external_output_override -isnot [bool] -or
        $Expected.external_output_override -isnot [bool] -or
        $Metadata.external_output_override -ne $Expected.external_output_override) {
        throw 'Checkpoint metadata external-output contract failed.'
    }
    foreach ($name in @('schema', 'aggregate_sha256')) {
        $actual = $Metadata.source_fingerprint.$name
        $wanted = $Expected.source_fingerprint.$name
        if ($actual -isnot [string] -or $wanted -isnot [string] -or $actual -cne $wanted) {
            throw "Checkpoint metadata source_fingerprint.$name mismatch."
        }
    }
    if ($Metadata.source_fingerprint.aggregate_sha256 -cnotmatch '^[A-F0-9]{64}$') {
        throw 'Checkpoint metadata source fingerprint hash is invalid.'
    }
    foreach ($name in @('file_count', 'total_bytes')) {
        $actual = $Metadata.source_fingerprint.$name
        $wanted = $Expected.source_fingerprint.$name
        if (-not (Test-CdrArchiveInteger $actual 1) -or
            -not (Test-CdrArchiveInteger $wanted 1) -or [uint64]$actual -ne [uint64]$wanted) {
            throw "Checkpoint metadata source_fingerprint.$name mismatch."
        }
    }
    foreach ($name in @('cdr-runtime.exe', 'cdr-offline-soak.exe', 'cdr-mcp-server.exe', 'cdr-pro-helper.exe')) {
        $actual = $Metadata.current_artifact_hashes.$name
        $wanted = $Expected.current_artifact_hashes.$name
        if ($actual.sha256 -isnot [string] -or $actual.sha256 -cnotmatch '^[A-F0-9]{64}$' -or
            $actual.sha256 -cne $wanted.sha256 -or
            -not (Test-CdrArchiveInteger $actual.bytes 1) -or
            [uint64]$actual.bytes -ne [uint64]$wanted.bytes -or
            $actual.archive_path -isnot [string] -or
            $actual.archive_path -cne "artifacts/$name") {
            throw "Checkpoint metadata artifact mismatch: $name"
        }
    }
    foreach ($name in @('offline_soak_evidence', 'workspace_gate_evidence')) {
        $actual = $Metadata.$name; $wanted = $Expected.$name
        if ($actual.filename -isnot [string] -or $actual.filename -cne $wanted.filename -or
            $actual.sha256 -isnot [string] -or $actual.sha256 -cnotmatch '^[A-F0-9]{64}$' -or
            $actual.sha256 -cne $wanted.sha256 -or
            $actual.status -isnot [string] -or $actual.status -cne $wanted.status) {
            throw "Checkpoint metadata evidence mismatch: $name"
        }
    }
    $database = $Metadata.database_snapshot; $wantedDatabase = $Expected.database_snapshot
    if ($database.sha256 -isnot [string] -or $database.sha256 -cnotmatch '^[A-F0-9]{64}$' -or
        $database.sha256 -cne $wantedDatabase.sha256 -or
        -not (Test-CdrArchiveInteger $database.bytes 1) -or
        [uint64]$database.bytes -ne [uint64]$wantedDatabase.bytes -or
        $database.archive_path -isnot [string] -or
        $database.archive_path -cne $wantedDatabase.archive_path) {
        throw 'Checkpoint metadata database snapshot mismatch.'
    }
    $rollback = $Metadata.source_rollback; $wantedRollback = $Expected.source_rollback
    if ($rollback.schema -isnot [string] -or $rollback.schema -cne $wantedRollback.schema -or
        $rollback.scope -isnot [string] -or
        $rollback.scope -cne 'local_windows_bot_and_operational_tooling' -or
        -not (Test-CdrArchiveInteger $rollback.source_file_count 1) -or
        [uint64]$rollback.source_file_count -ne [uint64]$wantedRollback.source_file_count) {
        throw 'Checkpoint metadata rollback contract mismatch.'
    }
}

function Assert-CdrCheckpointArchiveContract {
    [CmdletBinding()]
    param(
        [IO.FileStream]$ArchiveStream, [string]$ExtractRoot,
        [AllowNull()][object]$PayloadFiles, [AllowNull()][object]$ControlFiles,
        [Parameter(Mandatory = $true)][object]$ExpectedMetadata
    )
    $utf8 = [Text.UTF8Encoding]::new($false, $true)
    $payloadMap = New-CdrArchiveRecordMap $PayloadFiles 'Trusted staged payload'
    $controlMap = New-CdrArchiveRecordMap $ControlFiles 'Trusted staged control'
    if ($controlMap.Count -ne 2 -or -not $controlMap.ContainsKey('ARCHIVE-METADATA.json') -or
        -not $controlMap.ContainsKey('SHA256SUMS')) {
        throw 'Trusted staged control set must contain exactly metadata and SHA256SUMS.'
    }
    $expectedNames = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($name in $payloadMap.Keys) { $null = $expectedNames.Add($name) }
    foreach ($name in $controlMap.Keys) {
        if (-not $expectedNames.Add($name)) { throw "Trusted archive path collision: $name" }
    }
    Add-Type -AssemblyName System.IO.Compression
    $ArchiveStream.Position = 0
    $zip = [IO.Compression.ZipArchive]::new(
        $ArchiveStream, [IO.Compression.ZipArchiveMode]::Read, $true
    )
    try {
        $archiveNames = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        foreach ($entry in $zip.Entries) {
            $name = ([string]$entry.FullName).Replace('\', '/')
            Assert-CdrArchiveCanonicalPath $name 'Checkpoint ZIP entry name'
            if ($name.EndsWith('/') -or [string]::IsNullOrEmpty($entry.Name)) {
                throw "Checkpoint ZIP directory entries are not allowed: $name"
            }
            if (-not $archiveNames.Add($name)) { throw "Duplicate checkpoint ZIP entry: $name" }
        }
        if ($archiveNames.Count -ne $expectedNames.Count) {
            throw "Checkpoint ZIP entry count mismatch: expected=$($expectedNames.Count) actual=$($archiveNames.Count)"
        }
        foreach ($name in $expectedNames) {
            if (-not $archiveNames.Contains($name)) { throw "Checkpoint ZIP entry missing: $name" }
        }
    } finally { $zip.Dispose() }
    return [pscustomobject]@{
        PayloadMap = $payloadMap; ControlMap = $controlMap; ExpectedNames = $expectedNames
    }
}

function Assert-CdrCheckpointExtractedContract {
    [CmdletBinding()]
    param(
        [string]$ArchivePath, [string]$ExtractRoot, [object]$ArchiveContract,
        [Parameter(Mandatory = $true)][object]$ExpectedMetadata
    )
    $utf8 = [Text.UTF8Encoding]::new($false, $true)
    Expand-Archive -LiteralPath $ArchivePath -DestinationPath $ExtractRoot -Force
    $payloadMap = $ArchiveContract.PayloadMap
    $controlMap = $ArchiveContract.ControlMap
    $expectedNames = $ArchiveContract.ExpectedNames
    $metadataPath = Join-Path $ExtractRoot 'ARCHIVE-METADATA.json'
    $sumsPath = Join-Path $ExtractRoot 'SHA256SUMS'
    $metadata = [IO.File]::ReadAllText($metadataPath, $utf8) | ConvertFrom-Json
    Assert-CdrCheckpointMetadataContract $metadata $ExpectedMetadata
    $metadataMap = New-CdrArchiveRecordMap $metadata.payload_files 'Archive metadata payload'
    if ($metadataMap.Count -ne $payloadMap.Count) { throw 'Archive metadata payload count mismatch.' }
    foreach ($name in $payloadMap.Keys) {
        if (-not $metadataMap.ContainsKey($name) -or
            $metadataMap[$name].sha256 -cne $payloadMap[$name].sha256 -or
            [uint64]$metadataMap[$name].bytes -ne [uint64]$payloadMap[$name].bytes) {
            throw "Archive metadata differs from trusted staged payload: $name"
        }
    }
    $manifestMap = [Collections.Generic.SortedDictionary[string, string]]::new(
        [StringComparer]::Ordinal
    )
    foreach ($line in [IO.File]::ReadAllLines($sumsPath, $utf8)) {
        if ($line -notmatch '^([A-F0-9]{64})  (.+)$') { throw "Invalid SHA256SUMS line: $line" }
        $name = [string]$Matches[2]; Assert-CdrArchiveCanonicalPath $name 'SHA256SUMS path'
        if ($manifestMap.ContainsKey($name)) { throw "Duplicate SHA256SUMS path: $name" }
        $manifestMap.Add($name, [string]$Matches[1])
    }
    if ($manifestMap.Count -ne $payloadMap.Count) { throw 'SHA256SUMS payload count mismatch.' }
    foreach ($name in $payloadMap.Keys) {
        if (-not $manifestMap.ContainsKey($name) -or
            $manifestMap[$name] -cne $payloadMap[$name].sha256) {
            throw "SHA256SUMS differs from trusted staged payload: $name"
        }
        $path = Join-Path $ExtractRoot $name.Replace('/', '\')
        Assert-CdrCheckpointLeaf $path 'Extracted checkpoint payload'
        $item = Get-Item -LiteralPath $path
        if ((Get-CdrSha256 $path) -cne $payloadMap[$name].sha256 -or
            [uint64]$item.Length -ne [uint64]$payloadMap[$name].bytes) {
            throw "Extracted checkpoint differs from trusted staged payload: $name"
        }
    }
    foreach ($name in $controlMap.Keys) {
        $path = Join-Path $ExtractRoot $name
        Assert-CdrCheckpointLeaf $path 'Extracted checkpoint control'
        $item = Get-Item -LiteralPath $path
        if ((Get-CdrSha256 $path) -cne $controlMap[$name].sha256 -or
            [uint64]$item.Length -ne [uint64]$controlMap[$name].bytes) {
            throw "Extracted checkpoint control differs from trusted staged control: $name"
        }
    }
    $actualNames = @(Get-ChildItem -LiteralPath $ExtractRoot -Recurse -File | ForEach-Object {
        $_.FullName.Substring($ExtractRoot.Length).TrimStart('\').Replace('\', '/')
    })
    if ($actualNames.Count -ne $expectedNames.Count) { throw 'Extracted checkpoint name count mismatch.' }
    foreach ($name in $actualNames) {
        if (-not $expectedNames.Contains($name)) { throw "Unexpected extracted checkpoint file: $name" }
    }
    return [pscustomobject]@{ Metadata = $metadata; MetadataPath = $metadataPath; SumsPath = $sumsPath }
}

Export-ModuleMember -Function @(
    'Assert-CdrCheckpointArchiveContract', 'Assert-CdrCheckpointExtractedContract'
)
