function Get-MemoryAbNumericValue {
    param([AllowNull()]$Value, [Parameter(Mandatory = $true)][string]$Location)
    if ($null -eq $Value -or $Value -is [bool] -or $Value -isnot [ValueType]) {
        throw "comparison metric is not numeric: $Location"
    }
    try { $number = [double]$Value }
    catch { throw "comparison metric is not numeric: ${Location}: $($_.Exception.Message)" }
    if ([double]::IsNaN($number) -or [double]::IsInfinity($number)) {
        throw "comparison metric is not finite: $Location"
    }
    return $number
}

function Get-MemoryAbFiniteDelta {
    param([double]$Candidate, [double]$Baseline, [string]$Location)
    $delta = $Candidate - $Baseline
    if ([double]::IsNaN($delta) -or [double]::IsInfinity($delta)) {
        throw "comparison delta is not finite: $Location"
    }
    return [math]::Round($delta, 6)
}

function Get-MemoryAbSortedPropertyNames {
    param([AllowNull()]$Value, [Parameter(Mandatory = $true)][string]$Location)
    if ($null -eq $Value) { throw "comparison object is missing: $Location" }
    return @($Value.PSObject.Properties.Name | Sort-Object)
}

function Assert-MemoryAbSamePropertyNames {
    param([string[]]$BaselineNames, [string[]]$CandidateNames, [string]$Location)
    if (($BaselineNames -join "`n") -cne ($CandidateNames -join "`n")) {
        throw "comparison shape mismatch at $Location."
    }
}

function Assert-MemoryAbCompletePhaseSummary {
    param($Summary, [Parameter(Mandatory = $true)][string]$Role)
    if ($Summary.status -cne 'completed') { throw "$Role summary is not completed." }
    foreach ($field in @('measurement_id', 'runtime_label')) {
        if ([string]::IsNullOrWhiteSpace([string]$Summary.$field)) {
            throw "$Role summary $field is missing."
        }
    }
    if ([string]$Summary.phase -notin @('warm_idle', 'active', 'recovery')) {
        throw "$Role summary phase is invalid."
    }
    if ($Summary.readiness.ready_before_measurement -ne $true) {
        throw "$Role summary does not prove readiness before measurement."
    }
    if ($Summary.sampling.test_only_short_override -ne $false -or
        $Summary.sampling.test_only_non_discord_processes -ne $false) {
        throw "$Role summary is not a production measurement."
    }
    $duration = Get-MemoryAbNumericValue `
        -Value $Summary.sampling.requested_duration_seconds `
        -Location "$Role.sampling.requested_duration_seconds"
    $interval = Get-MemoryAbNumericValue `
        -Value $Summary.sampling.sample_interval_seconds `
        -Location "$Role.sampling.sample_interval_seconds"
    $actualDuration = Get-MemoryAbNumericValue `
        -Value $Summary.sampling.actual_duration_seconds `
        -Location "$Role.sampling.actual_duration_seconds"
    if ($duration -lt $script:MemoryAbProductionMinimumSeconds -or
        $interval -le 0 -or $actualDuration -lt $duration) {
        throw "$Role summary sampling duration or interval is invalid."
    }
    foreach ($identityName in @('bot', 'app_server')) {
        $identity = $Summary.identities.$identityName
        if ([int]$identity.pid -lt 1 -or
            [string]::IsNullOrWhiteSpace([string]$identity.started_at_utc) -or
            [string]::IsNullOrWhiteSpace([string]$identity.executable_path)) {
            throw "$Role summary $identityName identity is incomplete."
        }
    }
    if ($Summary.identities.app_server.parent_verified -ne $true -or
        [int]$Summary.identities.app_server.parent_bot_pid -ne
        [int]$Summary.identities.bot.pid) {
        throw "$Role summary app-server parent evidence is incomplete."
    }
    $expectedKinds = @('app-server', 'bot', 'combined')
    $actualKinds = Get-MemoryAbSortedPropertyNames `
        -Value $Summary.results -Location "$Role.results"
    Assert-MemoryAbSamePropertyNames $expectedKinds $actualKinds "$Role.results"
    $requiredMetrics = @(
        'cpu_machine_percent', 'cpu_one_core_percent', 'handle_count',
        'private_memory_bytes', 'thread_count', 'working_set_bytes'
    )
    $minimumSamples = [math]::Floor($duration / $interval)
    foreach ($kind in $expectedKinds) {
        $sampleCount = Get-MemoryAbNumericValue `
            -Value $Summary.results.$kind.sample_count `
            -Location "$Role.results.$kind.sample_count"
        if ($sampleCount -ne [math]::Truncate($sampleCount) -or
            $sampleCount -lt $minimumSamples) {
            throw "$Role summary sample_count is incomplete for $kind."
        }
        $actualMetrics = Get-MemoryAbSortedPropertyNames `
            -Value $Summary.results.$kind.metrics -Location "$Role.results.$kind.metrics"
        Assert-MemoryAbSamePropertyNames $requiredMetrics $actualMetrics `
            "$Role.results.$kind.metrics"
        foreach ($metric in $requiredMetrics) {
            $mean = Get-MemoryAbNumericValue -Value $Summary.results.$kind.metrics.$metric.mean `
                -Location "$Role.results.$kind.metrics.$metric.mean"
            $median = Get-MemoryAbNumericValue `
                -Value $Summary.results.$kind.metrics.$metric.median `
                -Location "$Role.results.$kind.metrics.$metric.median"
            $max = Get-MemoryAbNumericValue -Value $Summary.results.$kind.metrics.$metric.max `
                -Location "$Role.results.$kind.metrics.$metric.max"
            if ($mean -lt 0 -or $median -lt 0 -or $max -lt $mean -or $max -lt $median) {
                throw "$Role summary metric statistics are invalid at $kind.$metric."
            }
        }
    }
}
