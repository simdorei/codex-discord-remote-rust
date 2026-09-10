function Write-MemoryAbPhaseSummary {
    param([Parameter(Mandatory = $true)]$Context)
    $results = [ordered]@{}
    foreach ($kind in @('bot', 'app-server', 'combined')) {
        if ($Context.RecordsByKind[$kind].Count -gt 0) {
            $results[$kind] = New-MemoryAbRuntimeKindSummary `
                -Records @($Context.RecordsByKind[$kind])
        }
    }
    $appIdentity = $null
    if ($Context.HasApp) {
        $appIdentity = [ordered]@{
            pid = $AppServerPid
            started_at_utc = $AppServerStartedAtUtc.UtcDateTime.ToString('o')
            executable_path = $Context.AppExecutablePath
            parent_bot_pid = $BotPid
            parent_verified = $true
        }
    }
    $startupDurationMs = (
        $ReadyAtUtc.UtcDateTime - $BotStartedAtUtc.UtcDateTime
    ).TotalMilliseconds
    $summary = [ordered]@{
        schema = 'cdr.memory-ab.phase-summary.v1'
        status = 'completed'
        measurement_id = $script:MemoryAbMeasurementId
        runtime_label = $RuntimeLabel
        phase = $Phase
        pairing = [ordered]@{
            workload_id = $WorkloadId
            db_snapshot_id = $DbSnapshotId
            codex_version = $CodexVersion
        }
        identities = [ordered]@{
            bot = [ordered]@{
                pid = $BotPid
                started_at_utc = $BotStartedAtUtc.UtcDateTime.ToString('o')
                executable_path = $Context.BotExecutablePath
            }
            app_server = $appIdentity
        }
        startup = [ordered]@{
            source = 'operator_supplied_ready_timestamp'
            started_at_utc = $BotStartedAtUtc.UtcDateTime.ToString('o')
            ready_at_utc = $ReadyAtUtc.UtcDateTime.ToString('o')
            duration_milliseconds = [math]::Round($startupDurationMs, 3)
        }
        readiness = [ordered]@{
            source = 'operator_supplied_ready_timestamp'
            ready_at_utc = $ReadyAtUtc.UtcDateTime.ToString('o')
            measurement_started_at_utc = $Context.MeasurementStartedAt.ToString('o')
            ready_before_measurement = $true
            ready_for_seconds_before_measurement = [math]::Round(
                (
                    $Context.MeasurementStartedAt.UtcDateTime -
                    $ReadyAtUtc.UtcDateTime
                ).TotalSeconds,
                6
            )
        }
        sampling = [ordered]@{
            requested_duration_seconds = $DurationSeconds
            actual_duration_seconds = [math]::Round($Context.ActualDurationSeconds, 6)
            sample_interval_seconds = $SampleIntervalSeconds
            production_minimum_seconds = [int]$script:MemoryAbProductionMinimumSeconds
            test_only_short_override = $TestOnlyAllowShortDuration.IsPresent
            test_only_non_discord_processes = $TestOnlyAllowNonDiscordProcesses.IsPresent
            started_at_utc = $Context.MeasurementStartedAt.ToString('o')
            completed_at_utc = $Context.MeasurementCompletedAt.ToString('o')
        }
        results = $results
        outputs = [ordered]@{
            raw_jsonl = $Context.RawPath
            summary_json = $Context.SummaryPath
        }
    }
    Write-MemoryAbNewUtf8File `
        -Path $Context.SummaryPath -Content ($summary | ConvertTo-Json -Depth 20)
}
