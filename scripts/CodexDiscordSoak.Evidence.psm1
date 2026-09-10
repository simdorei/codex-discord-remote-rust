Set-StrictMode -Version Latest
function Throw-CodexSoakEvidenceError {
    param([string]$Code, [string]$Message)
    $exception = [InvalidOperationException]::new($Message)
    $exception.Data['CodexSoakFailureCode'] = $Code
    throw $exception
}
function Get-CodexSoakEvidencePropertyNames {
    param([AllowNull()][object]$InputObject, [string]$Code, [string]$Label)
    if ($null -eq $InputObject) { return @() }
    if ($InputObject -isnot [pscustomobject]) {
        Throw-CodexSoakEvidenceError $Code "$Label must be an inert PSCustomObject"
    }
    $properties = @($InputObject.PSObject.Properties)
    foreach ($property in $properties) {
        if ($property.MemberType -ne [Management.Automation.PSMemberTypes]::NoteProperty) { Throw-CodexSoakEvidenceError $Code "$Label properties must be inert NoteProperty members" }
    }
    @($properties | ForEach-Object { $_.Name })
}
function Get-CodexSoakEvidencePropertyValue {
    param([object]$InputObject, [string]$Name, [string]$Code, [string]$Label)
    if ($InputObject -isnot [pscustomobject]) {
        Throw-CodexSoakEvidenceError $Code "$Label must be an inert PSCustomObject"
    }
    $property = $InputObject.PSObject.Properties[$Name]
    if ($null -eq $property -or $property.MemberType -ne [Management.Automation.PSMemberTypes]::NoteProperty) {
        Throw-CodexSoakEvidenceError $Code "$Label is missing inert property '$Name'"
    }
    $value = $property.Value
    if ($null -ne $value) { $value = ([Management.Automation.PSObject]::AsPSObject($value)).PSObject.BaseObject }
    return ,$value
}
function Assert-CodexSoakEvidenceProperties {
    param([AllowNull()][object]$InputObject, [string[]]$Expected, [string]$Code, [string]$Label)
    if ($null -eq $InputObject) {
        Throw-CodexSoakEvidenceError $Code "$Label must be an object"
    }
    $names = @(Get-CodexSoakEvidencePropertyNames $InputObject $Code $Label)
    if ($names.Count -ne $Expected.Count) {
        Throw-CodexSoakEvidenceError $Code "$Label must contain exactly: $($Expected -join ', ')"
    }
    foreach ($name in $Expected) {
        if (-not ($names -ccontains $name)) {
            Throw-CodexSoakEvidenceError $Code "$Label is missing property '$name'"
        }
    }
}
function Test-CodexSoakUnsignedInteger {
    param([AllowNull()][object]$Value)
    $isInteger = $Value -is [byte] -or $Value -is [uint16] -or
        $Value -is [uint32] -or $Value -is [uint64] -or
        $Value -is [sbyte] -or $Value -is [int16] -or
        $Value -is [int32] -or $Value -is [int64]
    $isInteger -and [decimal]$Value -ge 0
}
function Convert-CodexSoakCanonicalRepoRoot {
    param([AllowNull()][object]$Value, [string]$Label)
    if ($Value -isnot [string] -or [string]::IsNullOrWhiteSpace($Value)) {
        Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "$Label.repo_root must be a non-empty string"
    }
    try {
        $windowsPath = ([string]$Value).Replace('/', '\')
        $isDriveFull = $windowsPath -match '^[A-Za-z]:\\'
        $isUncFull = $windowsPath.StartsWith('\\', [StringComparison]::Ordinal)
        if (-not ($isDriveFull -or $isUncFull) -or -not [IO.Path]::IsPathRooted($windowsPath)) {
            Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "$Label.repo_root must be an absolute path"
        }
        $full = [IO.Path]::GetFullPath($windowsPath)
        if (-not [IO.Directory]::Exists($full)) {
            Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "$Label.repo_root does not name an existing directory"
        }
        $full = ([IO.DirectoryInfo]::new($full)).FullName.Replace('/', '\')
        if ($full.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) {
            $full = '\\' + $full.Substring(8)
        } elseif ($full.StartsWith('\\?\', [StringComparison]::OrdinalIgnoreCase)) {
            $full = $full.Substring(4)
        }
        $root = [IO.Path]::GetPathRoot($full)
        while ($full.Length -gt $root.Length -and $full.EndsWith('\')) {
            $full = $full.Substring(0, $full.Length - 1)
        }
        $full
    } catch {
        $failureCode = $_.Exception.Data['CodexSoakFailureCode']; if ($failureCode -is [string] -and $failureCode -ceq 'fingerprint_record_invalid') { throw }
        Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "$Label.repo_root is invalid: $($_.Exception.Message)"
    }
}
function Assert-CodexSoakFingerprintHash {
    param([AllowNull()][object]$Value, [string]$Label)
    if ($Value -isnot [string] -or $Value -cnotmatch '^[0-9A-F]{64}$') {
        Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "$Label must be an uppercase SHA-256 string"
    }
}
function Assert-CodexSoakFingerprintRecord {
    param([AllowNull()][object]$Record, [string]$Label)
    $top = @('schema', 'repo_root', 'aggregate_sha256', 'file_count', 'total_bytes', 'files')
    Assert-CodexSoakEvidenceProperties $Record $top 'fingerprint_record_invalid' $Label
    $schema = Get-CodexSoakEvidencePropertyValue $Record 'schema' 'fingerprint_record_invalid' $Label
    if ($schema -isnot [string] -or $schema -cne 'cdr.rust-source-fingerprint.v1') {
        Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "$Label.schema must be cdr.rust-source-fingerprint.v1"
    }
    $rootValue = Get-CodexSoakEvidencePropertyValue $Record 'repo_root' 'fingerprint_record_invalid' $Label
    $root = Convert-CodexSoakCanonicalRepoRoot $rootValue $Label
    $aggregate = Get-CodexSoakEvidencePropertyValue $Record 'aggregate_sha256' 'fingerprint_record_invalid' $Label
    Assert-CodexSoakFingerprintHash $aggregate "$Label.aggregate_sha256"
    $fileCount = Get-CodexSoakEvidencePropertyValue $Record 'file_count' 'fingerprint_record_invalid' $Label
    $totalBytes = Get-CodexSoakEvidencePropertyValue $Record 'total_bytes' 'fingerprint_record_invalid' $Label
    if (-not (Test-CodexSoakUnsignedInteger $fileCount) -or
        -not (Test-CodexSoakUnsignedInteger $totalBytes)) {
        Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "$Label counts must be unsigned integers"
    }
    $filesValue = Get-CodexSoakEvidencePropertyValue $Record 'files' 'fingerprint_record_invalid' $Label
    if ($filesValue -isnot [object[]]) {
        Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "$Label.files must be a one-dimensional array"
    }
    $files = @($filesValue); [uint64]$sum = 0; $previousPath = $null
    $snapshots = [Collections.Generic.List[object]]::new()
    $seenPaths = [Collections.Generic.Dictionary[string, string]]::new([StringComparer]::OrdinalIgnoreCase)
    $strictUtf8 = [Text.UTF8Encoding]::new($false, $true)
    $invalidNameChars = [IO.Path]::GetInvalidFileNameChars()
    if ([uint64]$fileCount -ne [uint64]$files.Count) {
        Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "$Label.file_count does not match files"
    }
    for ($index = 0; $index -lt $files.Count; $index++) {
        $rowLabel = "$Label.files[$index]"
        Assert-CodexSoakEvidenceProperties $files[$index] @('path', 'bytes', 'sha256') 'fingerprint_record_invalid' $rowLabel
        $path = Get-CodexSoakEvidencePropertyValue $files[$index] 'path' 'fingerprint_record_invalid' $rowLabel
        $bytes = Get-CodexSoakEvidencePropertyValue $files[$index] 'bytes' 'fingerprint_record_invalid' $rowLabel
        $hash = Get-CodexSoakEvidencePropertyValue $files[$index] 'sha256' 'fingerprint_record_invalid' $rowLabel
        if ($path -isnot [string] -or [string]::IsNullOrEmpty($path)) {
            Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "$rowLabel.path must be a non-empty string"
        }; $path = [string]$path
        try {
            [void]$strictUtf8.GetBytes($path)
            $segments = @($path.Split('/'))
            if ([IO.Path]::IsPathRooted($path) -or $path.Contains('\')) {
                Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "$rowLabel.path must be a canonical relative logical path"
            }
            foreach ($segment in $segments) {
                if ($segment -eq '' -or $segment -eq '.' -or $segment -eq '..' -or
                    $segment.IndexOfAny($invalidNameChars) -ge 0) {
                    Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "$rowLabel.path contains an invalid segment"
                }
            }
        } catch {
            if ($_.Exception.Data['CodexSoakFailureCode']) { throw }
            Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "$rowLabel.path is invalid: $($_.Exception.Message)"
        }
        if ($seenPaths.ContainsKey($path)) {
            Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "$rowLabel.path duplicates $($seenPaths[$path])"
        }
        if ($null -ne $previousPath -and [string]::CompareOrdinal($previousPath, $path) -ge 0) {
            Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "$rowLabel.path is not in strict ordinal order"
        }
        $seenPaths.Add($path, $path)
        $previousPath = $path
        if (-not (Test-CodexSoakUnsignedInteger $bytes)) {
            Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "$rowLabel.bytes must be an unsigned integer"
        }
        Assert-CodexSoakFingerprintHash $hash "$rowLabel.sha256"
        if ([uint64]::MaxValue - $sum -lt [uint64]$bytes) {
            Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "$Label.total_bytes overflowed"
        }
        $sum += [uint64]$bytes
        $snapshots.Add([pscustomobject]@{
                Path = [string]$path; Bytes = [uint64]$bytes; Sha256 = [string]$hash
            })
    }
    if ($sum -ne [uint64]$totalBytes) {
        Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "$Label.total_bytes does not match files"
    }
    [pscustomobject]@{
        Schema = [string]$schema; Root = [string]$root; Aggregate = [string]$aggregate
        FileCount = [uint64]$fileCount; TotalBytes = [uint64]$totalBytes
        Files = [object[]]$snapshots.ToArray()
    }
}
function Test-CodexSoakSourceFingerprintEqual {
    param([Parameter(Mandatory = $true)][AllowNull()][object]$Expected,
        [Parameter(Mandatory = $true)][AllowNull()][object]$Actual)
    try {
        $left = Assert-CodexSoakFingerprintRecord $Expected 'Expected fingerprint'
        $right = Assert-CodexSoakFingerprintRecord $Actual 'Actual fingerprint'
        if (-not [string]::Equals($left.Schema, $right.Schema, [StringComparison]::Ordinal) -or
            -not [string]::Equals($left.Root, $right.Root, [StringComparison]::OrdinalIgnoreCase) -or
            -not [string]::Equals($left.Aggregate, $right.Aggregate, [StringComparison]::Ordinal) -or
            $left.FileCount -ne $right.FileCount -or $left.TotalBytes -ne $right.TotalBytes -or
            $left.Files.Count -ne $right.Files.Count) { return $false }
        for ($index = 0; $index -lt $left.Files.Count; $index++) {
            $a = $left.Files[$index]; $b = $right.Files[$index]
            if (-not [string]::Equals($a.Path, $b.Path, [StringComparison]::Ordinal) -or
                $a.Bytes -ne $b.Bytes -or
                -not [string]::Equals($a.Sha256, $b.Sha256, [StringComparison]::Ordinal)) { return $false }
        }
        $true
    } catch {
        $failureCode = $_.Exception.Data['CodexSoakFailureCode']; if ($failureCode -is [string] -and $failureCode -ceq 'fingerprint_record_invalid') { throw }
        Throw-CodexSoakEvidenceError 'fingerprint_record_invalid' "Fingerprint comparison failed: $($_.Exception.Message)"
    }
}
function Get-CodexSoakFinalEligibility {
    param([Parameter(Mandatory = $true)][AllowNull()][object]$OperationalStatus,
        [Parameter(Mandatory = $true)][AllowNull()][object]$Checks)
    try {
        $names = @(
        'same_run_canonical_release_build', 'canonical_release_harness',
        'build_fingerprints_equal', 'soak_fingerprints_equal', 'harness_provenance_verified',
        'harness_process_exit_confirmed', 'child_exit_zero', 'harness_contract_passed',
        'event_stream_valid', 'minimum_duration_met', 'harness_duration_matches_request',
        'harness_elapsed_reached_request', 'canonical_warmup', 'sampling_interval_not_weakened',
        'slope_limit_not_weakened', 'regression_samples_met', 'memory_slope_passed',
        'disabled_marker_preserved', 'cleanup_clean'
    )
    if ($OperationalStatus -isnot [string] -or -not (@('passed', 'integrity_failed', 'runtime_failed') -ccontains $OperationalStatus)) {
        Throw-CodexSoakEvidenceError 'eligibility_input_invalid' 'OperationalStatus must be passed, integrity_failed, or runtime_failed'
    }
    Assert-CodexSoakEvidenceProperties $Checks $names 'eligibility_input_invalid' 'Checks'
    $operationalPassed = $OperationalStatus -ceq 'passed'
    $orderedChecks = [ordered]@{ operational_passed = $operationalPassed }
    $reasons = [Collections.Generic.List[string]]::new()
    if (-not $operationalPassed) { $reasons.Add($OperationalStatus) }
    foreach ($name in $names) {
        $value = Get-CodexSoakEvidencePropertyValue $Checks $name 'eligibility_input_invalid' 'Checks'
        if ($value -isnot [bool]) {
            Throw-CodexSoakEvidenceError 'eligibility_input_invalid' "Checks.$name must be a Boolean"
        }
        $orderedChecks[$name] = $value
        if (-not $value) { $reasons.Add($name) }
    }
    $eligible = $operationalPassed -and $reasons.Count -eq 0
    $status = if ($eligible) { 'eligible' } elseif ($operationalPassed) { 'ineligible' } else { 'failed' }
        [pscustomobject][ordered]@{
            schema = 'cdr.windows-soak.final-eligibility.v1'; status = $status; eligible = [bool]$eligible
            reasons = [string[]]$reasons.ToArray()
            requirements = [pscustomobject][ordered]@{
                minimum_duration_seconds = [long]86400; warmup_seconds = [long]3600
                maximum_sample_interval_seconds = [long]60; maximum_slope_bytes_per_hour = [long]1048576
                minimum_regression_samples = [int]2
            }
            checks = [pscustomobject]$orderedChecks
        }
    } catch {
        $failureCode = $_.Exception.Data['CodexSoakFailureCode']; if ($failureCode -is [string] -and $failureCode -ceq 'eligibility_input_invalid') { throw }
        Throw-CodexSoakEvidenceError 'eligibility_input_invalid' "Eligibility evaluation failed: $($_.Exception.Message)"
    }
}
Export-ModuleMember -Function @(
    'Test-CodexSoakSourceFingerprintEqual', 'Get-CodexSoakFinalEligibility'
)
