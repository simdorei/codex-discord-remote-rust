Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.EvidenceShape.psm1') -ErrorAction Stop
Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.Payload.psm1') -ErrorAction Stop
Import-Module (Join-Path $PSScriptRoot 'CodexDiscordSoak.SourceFingerprint.psm1') -ErrorAction Stop
Set-StrictMode -Version Latest

function Assert-CdrQualityEvidenceContract([object]$Record) {
    Assert-CdrEvidenceCondition (Test-CdrEvidenceExactString $Record.text_scope 'rust_and_checkpoint_rollback_sources') 'quality text scope is invalid'
    foreach ($name in @('rust_files_checked', 'text_files_checked')) {
        Assert-CdrEvidenceCondition ((Test-CdrEvidenceInteger $Record.$name) -and $Record.$name -gt 0) "quality.$name must be positive"
    }
    foreach ($name in @('rust_invalid_utf8_count', 'text_invalid_utf8_count')) {
        Assert-CdrEvidenceCondition ((Test-CdrEvidenceInteger $Record.$name) -and $Record.$name -eq 0) "quality.$name must be zero"
    }
    foreach ($name in @('rust_utf8_bom_count', 'text_utf8_bom_count', 'production_rust_files_over_250_lines')) {
        Assert-CdrEvidenceCondition ((Test-CdrEvidenceInteger $Record.$name) -and $Record.$name -ge 0) "quality.$name must be a nonnegative integer"
    }
    Assert-CdrEvidenceCondition ($Record.exceptions -is [array]) 'quality.exceptions must be an array'
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($entry in $Record.exceptions) {
        Assert-CdrEvidenceCondition ($entry -is [pscustomobject] -and $entry -isnot [array] -and
            $entry.path -is [string] -and $entry.path -cmatch '^[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)+$' -and
            $entry.path -notmatch '(^|/)\.\.?(/|$)' -and $seen.Add($entry.path)) 'quality exception path is invalid or duplicated'
        Assert-CdrEvidenceCondition ($entry.bom -is [bool] -and
            (Test-CdrEvidenceInteger $entry.lines) -and $entry.lines -gt 0 -and
            ($entry.bom -or $entry.lines -gt 250)) 'quality exception measurements are invalid'
        Assert-CdrEvidenceCondition ($entry.reason -is [string] -and
            -not [string]::IsNullOrWhiteSpace($entry.reason)) 'quality exception requires a reviewed reason'
    }
}

function Get-CdrCheckpointQualityRecord([string]$RepoRoot) {
    $root = [IO.Path]::GetFullPath($RepoRoot)
    $source = Get-CodexSoakSourceFingerprint -RepoRoot $root
    $paths = [Collections.Generic.SortedSet[string]]::new([StringComparer]::Ordinal)
    foreach ($file in $source.files) {
        if ($file.path.EndsWith('.rs', [StringComparison]::OrdinalIgnoreCase)) { $null = $paths.Add($file.path) }
    }
    foreach ($file in @(Get-CdrCheckpointRollbackSourceEntries -RepoRoot $root)) { $null = $paths.Add($file.source_path) }
    $policyPath = Join-Path $root 'scripts/RustMigrationCheckpoint.QualityApprovals.json'
    $utf8 = [Text.UTF8Encoding]::new($false, $true)
    $policy = $utf8.GetString([IO.File]::ReadAllBytes($policyPath)) | ConvertFrom-Json
    if ($policy.schema -cne 'cdr.reviewed-quality-exceptions.v1' -or $policy.exceptions -isnot [array]) {
        throw 'quality approval policy schema is invalid'
    }
    $approvals = [Collections.Generic.Dictionary[string, object]]::new([StringComparer]::Ordinal)
    foreach ($entry in $policy.exceptions) {
        if ($approvals.ContainsKey($entry.path)) { throw 'quality approval policy contains duplicate paths' }
        $approvals.Add($entry.path, $entry)
    }
    $record = [pscustomobject][ordered]@{
        text_scope = 'rust_and_checkpoint_rollback_sources'; rust_files_checked = 0; text_files_checked = $paths.Count
        rust_utf8_bom_count = 0; rust_invalid_utf8_count = 0; production_rust_files_over_250_lines = 0
        text_utf8_bom_count = 0; text_invalid_utf8_count = 0; exceptions = @()
    }
    $exceptions = [Collections.Generic.List[object]]::new()
    foreach ($path in $paths) {
        $bytes = [IO.File]::ReadAllBytes((Join-Path $root $path))
        $rust = $path.EndsWith('.rs', [StringComparison]::OrdinalIgnoreCase)
        if ($rust) { $record.rust_files_checked++ }
        try { $text = $utf8.GetString($bytes) } catch [Text.DecoderFallbackException] {
            $record.text_invalid_utf8_count++
            if ($rust) { $record.rust_invalid_utf8_count++ }
            continue
        }
        $bom = $bytes.Length -ge 3 -and $bytes[0] -eq 239 -and $bytes[1] -eq 187 -and $bytes[2] -eq 191
        $lines = if ($text.Length -eq 0) { 0 } else {
            $count = [regex]::Split($text, '\r\n|\n|\r').Count
            if ($text.EndsWith("`n") -or $text.EndsWith("`r")) { $count-- }; $count
        }
        $production = $rust -and $path -match '/src/' -and $path -notmatch '(^|/)(tests|[^/]*_tests)(/|\.rs$)'
        if ($bom) { $record.text_utf8_bom_count++; if ($rust) { $record.rust_utf8_bom_count++ } }
        if ($production -and $lines -gt 250) { $record.production_rust_files_over_250_lines++ }
        if ($bom -or ($production -and $lines -gt 250)) {
            $reason = if ($approvals.ContainsKey($path)) { $approvals[$path].reason } else { '' }
            $exceptions.Add([pscustomobject][ordered]@{ path = $path; bom = [bool]$bom; lines = [int]$lines; reason = $reason })
        }
    }
    $record.exceptions = [object[]]$exceptions.ToArray()
    Assert-CdrQualityEvidenceContract $record
    if ($approvals.Count -ne $exceptions.Count) { throw 'quality approval set does not match measured exceptions' }
    foreach ($actual in $exceptions) {
        $approved = $approvals[$actual.path]
        if ($approved.bom -isnot [bool] -or $actual.bom -ne $approved.bom -or
            -not (Test-CdrEvidenceInteger $approved.lines) -or $actual.lines -ne $approved.lines -or
            $actual.reason -cne $approved.reason) { throw "quality approval measurements differ: $($actual.path)" }
    }
    return $record
}

function Assert-CdrCheckpointQualityRecord([object]$Record, [string]$RepoRoot) {
    Assert-CdrQualityEvidenceContract $Record
    $expected = Get-CdrCheckpointQualityRecord -RepoRoot $RepoRoot
    foreach ($field in @('text_scope', 'rust_files_checked', 'text_files_checked', 'rust_utf8_bom_count',
        'rust_invalid_utf8_count', 'text_utf8_bom_count', 'text_invalid_utf8_count', 'production_rust_files_over_250_lines')) {
        if ($Record.$field -cne $expected.$field) { throw "quality measurement mismatch: $field" }
    }
    if ($Record.exceptions.Count -ne $expected.exceptions.Count) { throw 'quality exception count mismatch' }
    for ($i = 0; $i -lt $expected.exceptions.Count; $i++) {
        foreach ($field in @('path', 'bom', 'lines', 'reason')) {
            if ($Record.exceptions[$i].$field -cne $expected.exceptions[$i].$field) { throw "quality exception mismatch: $field at index $i" }
        }
    }
}

Export-ModuleMember -Function 'Get-CdrCheckpointQualityRecord', 'Assert-CdrQualityEvidenceContract', 'Assert-CdrCheckpointQualityRecord'
