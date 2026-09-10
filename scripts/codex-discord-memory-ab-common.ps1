function Resolve-MemoryAbFullPath {
    param([Parameter(Mandatory = $true)][string]$Path)
    return [IO.Path]::GetFullPath($Path)
}

function Ensure-MemoryAbParentDirectory {
    param([Parameter(Mandatory = $true)][string]$Path)
    $parent = [IO.Path]::GetDirectoryName($Path)
    if (-not [string]::IsNullOrWhiteSpace($parent)) {
        [void][IO.Directory]::CreateDirectory($parent)
    }
}

function Assert-MemoryAbOutputAvailable {
    param([Parameter(Mandatory = $true)][string]$Path)
    if ([IO.File]::Exists($Path) -or [IO.Directory]::Exists($Path)) {
        throw "output path already exists; refusing to mix or overwrite evidence: $Path"
    }
}

function New-MemoryAbEmptyOutputFile {
    param([Parameter(Mandatory = $true)][string]$Path)
    $stream = [IO.File]::Open(
        $Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read
    )
    $stream.Dispose()
}

function Write-MemoryAbNewUtf8File {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Content
    )
    $bytes = $script:MemoryAbUtf8NoBom.GetBytes($Content)
    $stream = [IO.File]::Open(
        $Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read
    )
    try {
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush()
    } finally {
        $stream.Dispose()
    }
}

function New-MemoryAbRawRecord {
    param(
        [Parameter(Mandatory = $true)][int]$SampleIndex,
        [Parameter(Mandatory = $true)][double]$ElapsedSeconds,
        [Parameter(Mandatory = $true)][string]$PhaseName,
        [Parameter(Mandatory = $true)][string]$RuntimeLabel,
        [Parameter(Mandatory = $true)][string]$RuntimeKind,
        [Parameter(Mandatory = $true)][long]$WorkingSetBytes,
        [Parameter(Mandatory = $true)][long]$PrivateMemoryBytes,
        [Parameter(Mandatory = $true)][double]$CpuOneCorePercent,
        [Parameter(Mandatory = $true)][int]$HandleCount,
        [Parameter(Mandatory = $true)][int]$ThreadCount,
        [Parameter(Mandatory = $true)][int]$LogicalProcessorCount
    )
    return [ordered]@{
        schema = 'cdr.memory-ab.raw-sample.v1'
        measurement_id = $script:MemoryAbMeasurementId
        captured_at_utc = [DateTimeOffset]::UtcNow.ToString('o')
        sample_index = $SampleIndex
        elapsed_seconds = [math]::Round($ElapsedSeconds, 6)
        phase = $PhaseName
        runtime_label = $RuntimeLabel
        workload_id = $WorkloadId
        db_snapshot_id = $DbSnapshotId
        codex_version = $CodexVersion
        bot_pid = $BotPid
        bot_started_at_utc = $BotStartedAtUtc.UtcDateTime.ToString('o')
        bot_executable_path = $script:MemoryAbBotExecutablePath
        app_server_pid = $script:MemoryAbAppServerPid
        app_server_started_at_utc = $script:MemoryAbAppServerStartedAtUtc
        app_server_executable_path = $script:MemoryAbAppServerExecutablePath
        runtime_kind = $RuntimeKind
        working_set_bytes = $WorkingSetBytes
        private_memory_bytes = $PrivateMemoryBytes
        cpu_one_core_percent = $CpuOneCorePercent
        cpu_machine_percent = [math]::Round(
            $CpuOneCorePercent / [math]::Max(1, $LogicalProcessorCount), 6
        )
        handle_count = $HandleCount
        thread_count = $ThreadCount
    }
}

function Write-MemoryAbRawRecord {
    param(
        [Parameter(Mandatory = $true)][Collections.IDictionary]$Record,
        [Parameter(Mandatory = $true)][string]$Path
    )
    $line = $Record | ConvertTo-Json -Compress -Depth 5
    [IO.File]::AppendAllText($Path, "$line`r`n", $script:MemoryAbUtf8NoBom)
}

function Get-MemoryAbStatisticSummary {
    param(
        [Parameter(Mandatory = $true)][object[]]$Records,
        [Parameter(Mandatory = $true)][string]$PropertyName
    )
    $values = @(
        $Records |
            ForEach-Object { [double]$_.PSObject.Properties[$PropertyName].Value } |
            Sort-Object
    )
    if ($values.Count -eq 0) { throw "cannot summarize empty metric $PropertyName." }
    $sum = 0.0
    foreach ($value in $values) { $sum += $value }
    $middle = [int][math]::Floor($values.Count / 2)
    if (($values.Count % 2) -eq 1) {
        $median = $values[$middle]
    } else {
        $median = ($values[$middle - 1] + $values[$middle]) / 2.0
    }
    return [ordered]@{
        mean = [math]::Round($sum / $values.Count, 6)
        median = [math]::Round($median, 6)
        max = [math]::Round($values[$values.Count - 1], 6)
    }
}

function New-MemoryAbRuntimeKindSummary {
    param([Parameter(Mandatory = $true)][object[]]$Records)
    $metrics = [ordered]@{}
    foreach ($metric in @(
        'working_set_bytes', 'private_memory_bytes', 'cpu_one_core_percent',
        'cpu_machine_percent', 'handle_count', 'thread_count'
    )) {
        $metrics[$metric] = Get-MemoryAbStatisticSummary `
            -Records $Records -PropertyName $metric
    }
    return [ordered]@{ sample_count = $Records.Count; metrics = $metrics }
}
