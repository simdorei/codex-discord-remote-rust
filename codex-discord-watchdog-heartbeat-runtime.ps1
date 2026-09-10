function Get-WatchdogHeartbeatIssue {
    param(
        [string]$HeartbeatPath,
        [int]$MaxAgeSeconds,
        [int]$StartupGraceSeconds,
        [datetime]$Now = (Get-Date),
        [datetime]$ProcessStartedAt = [datetime]::MinValue,
        [Nullable[double]]$ProcessAgeSeconds = $null
    )

    if ($MaxAgeSeconds -le 0 -or -not $HeartbeatPath) {
        return ""
    }

    if (Test-Path -LiteralPath $HeartbeatPath) {
        $heartbeatAgeSeconds = [math]::Max(
            0,
            [math]::Floor(($Now - (Get-Item -LiteralPath $HeartbeatPath).LastWriteTime).TotalSeconds)
        )
        if ($heartbeatAgeSeconds -ge $MaxAgeSeconds) {
            return (
                "heartbeat_age_seconds=$heartbeatAgeSeconds " +
                "threshold=$MaxAgeSeconds"
            )
        }
        return ""
    }

    $graceSeconds = [math]::Max(0, $StartupGraceSeconds)
    if ($ProcessAgeSeconds -eq $null -and $ProcessStartedAt -ne [datetime]::MinValue) {
        $ProcessAgeSeconds = [math]::Max(
            0,
            [math]::Floor(($Now - $ProcessStartedAt).TotalSeconds)
        )
    }
    if ($ProcessAgeSeconds -eq $null) {
        return "heartbeat_missing process_age_seconds=unknown startup_grace_seconds=$graceSeconds"
    }
    $boundedProcessAgeSeconds = [math]::Max(0, [math]::Floor($ProcessAgeSeconds))
    if ($boundedProcessAgeSeconds -ge $graceSeconds) {
        return (
            "heartbeat_missing process_age_seconds=$boundedProcessAgeSeconds " +
            "startup_grace_seconds=$graceSeconds"
        )
    }
    return ""
}
