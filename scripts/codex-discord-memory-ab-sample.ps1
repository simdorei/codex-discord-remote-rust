function Invoke-MemoryAbPhaseSampling {
    if (-not $AuthorizedLiveMeasurement.IsPresent) {
        throw 'Sampling requires explicit -AuthorizedLiveMeasurement.'
    }
    if ($DurationSeconds -lt $script:MemoryAbProductionMinimumSeconds -and
        -not $TestOnlyAllowShortDuration.IsPresent) {
        throw 'Production memory phase duration must be at least 300 seconds.'
    }
    if ($TestOnlyAllowNonDiscordProcesses.IsPresent -and
        -not $TestOnlyAllowShortDuration.IsPresent) {
        throw 'TestOnlyAllowNonDiscordProcesses requires TestOnlyAllowShortDuration.'
    }
    $hasAppPid = $script:MemoryAbInvocationParameters.ContainsKey('AppServerPid')
    $hasAppStart = $script:MemoryAbInvocationParameters.ContainsKey('AppServerStartedAtUtc')
    $hasAppExecutable = $script:MemoryAbInvocationParameters.ContainsKey(
        'AppServerExecutablePath'
    )
    if (($hasAppPid -or $hasAppStart -or $hasAppExecutable) -and
        -not ($hasAppPid -and $hasAppStart -and $hasAppExecutable)) {
        throw (
            'AppServerPid, AppServerStartedAtUtc, and AppServerExecutablePath ' +
            'must be supplied together.'
        )
    }
    $hasApp = $hasAppPid -and $hasAppStart -and $hasAppExecutable
    if (-not $TestOnlyAllowShortDuration.IsPresent -and -not $hasApp) {
        throw 'Production sampling requires an explicit app-server PID, start time, and path.'
    }
    if ($hasApp -and $AppServerPid -eq $BotPid) {
        throw 'BotPid and AppServerPid must identify different processes.'
    }
    if ($ReadyAtUtc.UtcDateTime.Ticks -lt $BotStartedAtUtc.UtcDateTime.Ticks) {
        throw 'ReadyAtUtc cannot be earlier than BotStartedAtUtc.'
    }
    if ($hasApp -and
        $ReadyAtUtc.UtcDateTime.Ticks -lt $AppServerStartedAtUtc.UtcDateTime.Ticks) {
        throw 'ReadyAtUtc cannot be earlier than AppServerStartedAtUtc.'
    }

    $botExecutableFullPath = Resolve-MemoryAbFullPath -Path $BotExecutablePath
    if (-not (Test-Path -LiteralPath $botExecutableFullPath -PathType Leaf)) {
        throw "BotExecutablePath was not found: $botExecutableFullPath"
    }
    $appExecutableFullPath = $null
    if ($hasApp) {
        $appExecutableFullPath = Resolve-MemoryAbFullPath -Path $AppServerExecutablePath
        if (-not (Test-Path -LiteralPath $appExecutableFullPath -PathType Leaf)) {
            throw "AppServerExecutablePath was not found: $appExecutableFullPath"
        }
    }

    $bot = Get-MemoryAbVerifiedProcess -ProcessId $BotPid `
        -ExpectedStartUtc $BotStartedAtUtc `
        -ExpectedExecutablePath $botExecutableFullPath -ProcessKind 'bot'
    try {
        if ($hasApp) {
            $app = Get-MemoryAbVerifiedProcess -ProcessId $AppServerPid `
                -ExpectedStartUtc $AppServerStartedAtUtc `
                -ExpectedExecutablePath $appExecutableFullPath -ProcessKind 'app-server'
            try {
                Assert-MemoryAbAppServerParent `
                    -ProcessId $AppServerPid -ExpectedParentPid $BotPid
            } finally {
                $app.Dispose()
            }
        }
    } finally {
        $bot.Dispose()
    }
    if (-not $TestOnlyAllowNonDiscordProcesses.IsPresent) {
        Assert-MemoryAbNoSecondLiveDiscordBot -ExpectedBotPid $BotPid
    }

    $rawPath = Resolve-MemoryAbFullPath -Path $RawJsonlPath
    $summaryFullPath = Resolve-MemoryAbFullPath -Path $SummaryPath
    if ($rawPath.Equals($summaryFullPath, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'RawJsonlPath and SummaryPath must be different files.'
    }
    Assert-MemoryAbOutputAvailable -Path $rawPath
    Assert-MemoryAbOutputAvailable -Path $summaryFullPath
    $measurementStartedAt = [DateTimeOffset]::UtcNow
    if ($ReadyAtUtc.UtcDateTime.Ticks -gt $measurementStartedAt.UtcDateTime.Ticks) {
        throw 'ReadyAtUtc cannot be later than the measurement start time.'
    }
    Ensure-MemoryAbParentDirectory -Path $rawPath
    Ensure-MemoryAbParentDirectory -Path $summaryFullPath
    New-MemoryAbEmptyOutputFile -Path $rawPath

    $script:MemoryAbMeasurementId = [guid]::NewGuid().ToString('N')
    $script:MemoryAbBotExecutablePath = $botExecutableFullPath
    $script:MemoryAbAppServerPid = $(if ($hasApp) { $AppServerPid } else { $null })
    $script:MemoryAbAppServerStartedAtUtc = $(
        if ($hasApp) { $AppServerStartedAtUtc.UtcDateTime.ToString('o') } else { $null }
    )
    $script:MemoryAbAppServerExecutablePath = $appExecutableFullPath

    $logicalProcessorCount = [math]::Max(1, [Environment]::ProcessorCount)
    $recordsByKind = [ordered]@{
        bot = [Collections.Generic.List[object]]::new()
        'app-server' = [Collections.Generic.List[object]]::new()
        combined = [Collections.Generic.List[object]]::new()
    }
    $previousBotCpu = $null
    $previousAppCpu = $null
    $previousElapsed = $null
    $sampleIndex = 0
    $clock = [Diagnostics.Stopwatch]::StartNew()

    while ($true) {
        if ($sampleIndex -gt 0) {
            $scheduledAt = [math]::Min(
                $DurationSeconds, $sampleIndex * $SampleIntervalSeconds
            )
            $delaySeconds = $scheduledAt - $clock.Elapsed.TotalSeconds
            if ($delaySeconds -gt 0) {
                Start-Sleep -Milliseconds ([math]::Max(
                    1, [int][math]::Ceiling($delaySeconds * 1000.0)
                ))
            }
        }
        $elapsed = [double]$clock.Elapsed.TotalSeconds
        $elapsedDelta = if ($null -eq $previousElapsed) {
            0.0
        } else {
            [math]::Max(0.0, $elapsed - [double]$previousElapsed)
        }
        if (-not $TestOnlyAllowNonDiscordProcesses.IsPresent) {
            Assert-MemoryAbNoSecondLiveDiscordBot -ExpectedBotPid $BotPid
        }
        $botProcess = Get-MemoryAbVerifiedProcess -ProcessId $BotPid `
            -ExpectedStartUtc $BotStartedAtUtc `
            -ExpectedExecutablePath $botExecutableFullPath -ProcessKind 'bot'
        try { $botSnapshot = Get-MemoryAbProcessSnapshot -Process $botProcess }
        finally { $botProcess.Dispose() }
        $botCpu = Get-MemoryAbCpuOneCorePercent `
            -CurrentTotalSeconds $botSnapshot.CpuTotalSeconds `
            -PreviousTotalSeconds $previousBotCpu -ElapsedSeconds $elapsedDelta
        $botRecord = New-MemoryAbRawRecord -SampleIndex $sampleIndex `
            -ElapsedSeconds $elapsed -PhaseName $Phase -RuntimeLabel $RuntimeLabel `
            -RuntimeKind 'bot' -WorkingSetBytes $botSnapshot.WorkingSetBytes `
            -PrivateMemoryBytes $botSnapshot.PrivateMemoryBytes `
            -CpuOneCorePercent $botCpu -HandleCount $botSnapshot.HandleCount `
            -ThreadCount $botSnapshot.ThreadCount -LogicalProcessorCount $logicalProcessorCount
        Write-MemoryAbRawRecord -Record $botRecord -Path $rawPath
        $recordsByKind.bot.Add([pscustomobject]$botRecord)

        if ($hasApp) {
            $appProcess = Get-MemoryAbVerifiedProcess -ProcessId $AppServerPid `
                -ExpectedStartUtc $AppServerStartedAtUtc `
                -ExpectedExecutablePath $appExecutableFullPath -ProcessKind 'app-server'
            try {
                Assert-MemoryAbAppServerParent `
                    -ProcessId $AppServerPid -ExpectedParentPid $BotPid
                $appSnapshot = Get-MemoryAbProcessSnapshot -Process $appProcess
            } finally {
                $appProcess.Dispose()
            }
            $appCpu = Get-MemoryAbCpuOneCorePercent `
                -CurrentTotalSeconds $appSnapshot.CpuTotalSeconds `
                -PreviousTotalSeconds $previousAppCpu -ElapsedSeconds $elapsedDelta
            $appRecord = New-MemoryAbRawRecord -SampleIndex $sampleIndex `
                -ElapsedSeconds $elapsed -PhaseName $Phase -RuntimeLabel $RuntimeLabel `
                -RuntimeKind 'app-server' -WorkingSetBytes $appSnapshot.WorkingSetBytes `
                -PrivateMemoryBytes $appSnapshot.PrivateMemoryBytes `
                -CpuOneCorePercent $appCpu -HandleCount $appSnapshot.HandleCount `
                -ThreadCount $appSnapshot.ThreadCount `
                -LogicalProcessorCount $logicalProcessorCount
            Write-MemoryAbRawRecord -Record $appRecord -Path $rawPath
            $recordsByKind['app-server'].Add([pscustomobject]$appRecord)

            $combinedRecord = New-MemoryAbRawRecord -SampleIndex $sampleIndex `
                -ElapsedSeconds $elapsed -PhaseName $Phase -RuntimeLabel $RuntimeLabel `
                -RuntimeKind 'combined' `
                -WorkingSetBytes ($botSnapshot.WorkingSetBytes + $appSnapshot.WorkingSetBytes) `
                -PrivateMemoryBytes (
                    $botSnapshot.PrivateMemoryBytes + $appSnapshot.PrivateMemoryBytes
                ) `
                -CpuOneCorePercent ($botCpu + $appCpu) `
                -HandleCount ($botSnapshot.HandleCount + $appSnapshot.HandleCount) `
                -ThreadCount ($botSnapshot.ThreadCount + $appSnapshot.ThreadCount) `
                -LogicalProcessorCount $logicalProcessorCount
            Write-MemoryAbRawRecord -Record $combinedRecord -Path $rawPath
            $recordsByKind.combined.Add([pscustomobject]$combinedRecord)
            $previousAppCpu = [double]$appSnapshot.CpuTotalSeconds
        }

        $previousBotCpu = [double]$botSnapshot.CpuTotalSeconds
        $previousElapsed = $elapsed
        $sampleIndex += 1
        if ($clock.Elapsed.TotalSeconds -ge $DurationSeconds) { break }
    }
    $clock.Stop()
    $measurementCompletedAt = [DateTimeOffset]::UtcNow
    if (-not $TestOnlyAllowShortDuration.IsPresent) {
        $minimumSamples = [int][math]::Floor($DurationSeconds / $SampleIntervalSeconds)
        foreach ($kind in @('bot', 'app-server', 'combined')) {
            if ($recordsByKind[$kind].Count -lt $minimumSamples) {
                throw "production sample cadence was not sustained for runtime kind $kind."
            }
        }
    }
    $context = [pscustomobject]@{
        HasApp = $hasApp; RecordsByKind = $recordsByKind
        MeasurementStartedAt = $measurementStartedAt
        MeasurementCompletedAt = $measurementCompletedAt
        ActualDurationSeconds = [double]$clock.Elapsed.TotalSeconds
        BotExecutablePath = $botExecutableFullPath
        AppExecutablePath = $appExecutableFullPath
        RawPath = $rawPath; SummaryPath = $summaryFullPath
    }
    Write-MemoryAbPhaseSummary -Context $context
    Write-Output "phase_sample_written phase=$Phase summary=$summaryFullPath"
}
