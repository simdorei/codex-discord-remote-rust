if (-not (Get-Command Resolve-CodexRuntimePythonExecutable -ErrorAction SilentlyContinue)) {
    . (Join-Path $ScriptDir 'codex-discord-python-runtime.ps1')
}

function Get-CodexThreadUpdatedAt {
    param([string]$UpdatedAtText)

    if ([string]::IsNullOrWhiteSpace($UpdatedAtText)) {
        return $null
    }

    [datetime]$updatedAt = [datetime]::MinValue
    $formats = @(
        'yyyy-MM-dd HH:mm:ss',
        'yyyy-MM-ddTHH:mm:ssK',
        'yyyy-MM-ddTHH:mm:ss'
    )
    $parsed = [datetime]::TryParseExact(
        $UpdatedAtText,
        $formats,
        [Globalization.CultureInfo]::InvariantCulture,
        [Globalization.DateTimeStyles]::AssumeLocal,
        [ref]$updatedAt
    )
    if (-not $parsed) {
        $parsed = [datetime]::TryParse(
            $UpdatedAtText,
            [Globalization.CultureInfo]::InvariantCulture,
            [Globalization.DateTimeStyles]::AssumeLocal,
            [ref]$updatedAt
        )
    }
    if (-not $parsed) {
        return $null
    }
    return $updatedAt
}

function Get-CodexThreadRestartBlockers {
    param([int]$QuietSeconds = $RestartQuietSeconds)

    if (-not (Test-Path -LiteralPath $BridgePath)) {
        throw "Cannot verify Codex thread state before restart; bridge script not found: $BridgePath"
    }

    $pythonExecutable = Get-CodexRuntimePythonExecutable
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $bridgeOutput = & $pythonExecutable $BridgePath list --db-root --limit 0 2>&1
        $bridgeExitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    if ($bridgeExitCode -ne 0) {
        throw "Cannot verify Codex thread state before restart.`n$($bridgeOutput -join "`n")"
    }

    $blockers = @()
    $now = Get-Date
    foreach ($line in $bridgeOutput) {
        if ($line -notmatch '\|') {
            continue
        }
        $parts = [string]$line -split '\|'
        if ($parts.Count -lt 3) {
            continue
        }
        $state = $parts[2].Trim()
        if ($state -and $state -ne 'idle') {
            $blockers += [pscustomobject]@{
                Reason = 'busy'
                Detail = ([string]$line).Trim()
            }
            continue
        }

        if ($QuietSeconds -gt 0) {
            if ($parts.Count -lt 9) {
                $blockers += [pscustomobject]@{
                    Reason = 'unknown_timestamp'
                    Detail = ([string]$line).Trim()
                }
                continue
            }
            $updatedAtText = $parts[8].Trim()
            $updatedAt = Get-CodexThreadUpdatedAt -UpdatedAtText $updatedAtText
            if ($updatedAt -eq $null) {
                $blockers += [pscustomobject]@{
                    Reason = 'unknown_timestamp'
                    Detail = "{0} | updated_at={1}" -f ([string]$line).Trim(), $updatedAtText
                }
                continue
            }
            $ageSeconds = [math]::Floor(($now - $updatedAt).TotalSeconds)
            if ($ageSeconds -lt $QuietSeconds) {
                $blockers += [pscustomobject]@{
                    Reason = 'recent'
                    Detail = (
                        "{0} | updated_age_seconds={1} quiet_seconds={2}" -f `
                            ([string]$line).Trim(), $ageSeconds, $QuietSeconds
                    )
                }
                continue
            }
        }
    }

    return $blockers
}

function Format-CodexThreadRestartBlockers {
    param($Blockers)

    $lines = @()
    foreach ($blocker in $Blockers) {
        $lines += ("{0}: {1}" -f $blocker.Reason, $blocker.Detail)
    }
    return $lines
}

function Assert-CodexThreadsQuietForRestart {
    $blockers = @(Get-CodexThreadRestartBlockers -QuietSeconds $RestartQuietSeconds)
    if ($blockers.Count -gt 0) {
        $lines = Format-CodexThreadRestartBlockers -Blockers $blockers
        throw "Refusing to restart Codex Discord bot because Codex threads are busy or not quiet.`n$($lines -join "`n")"
    }
}

function Wait-CodexThreadsQuietForRestart {
    if ($RestartWaitTimeoutSeconds -le 0) {
        Assert-CodexThreadsQuietForRestart
        return
    }

    $deadline = (Get-Date).AddSeconds($RestartWaitTimeoutSeconds)
    $lastLogAt = [datetime]::MinValue
    while ($true) {
        $blockers = @(Get-CodexThreadRestartBlockers -QuietSeconds $RestartQuietSeconds)
        if ($blockers.Count -eq 0) {
            Write-LauncherLog "watchdog_restart_quiet quiet_seconds=$RestartQuietSeconds"
            return
        }

        $now = Get-Date
        if ($now -ge $deadline) {
            $lines = Format-CodexThreadRestartBlockers -Blockers $blockers
            throw "Timed out waiting for Codex threads to become quiet before restart.`n$($lines -join "`n")"
        }

        if (($now - $lastLogAt).TotalSeconds -ge 15) {
            $sample = ($blockers | Select-Object -First 1).Detail
            Write-LauncherLog "watchdog_restart_waiting quiet_seconds=$RestartQuietSeconds blockers=$($blockers.Count) sample=$sample"
            $lastLogAt = $now
        }
        Start-Sleep -Seconds $RestartWaitPollSeconds
    }
}

function Claim-RestartRequest {
    if (-not (Test-Path -LiteralPath $RestartRequestPath)) {
        return ""
    }
    try {
        Move-Item -LiteralPath $RestartRequestPath -Destination $RestartClaimPath -ErrorAction Stop
        Write-LauncherLog "watchdog_restart_claimed marker=$RestartRequestPath claim=$RestartClaimPath"
        return $RestartClaimPath
    } catch {
        if (-not (Test-Path -LiteralPath $RestartRequestPath)) {
            Write-LauncherLog "watchdog_restart_claim_lost marker=$RestartRequestPath"
            return ""
        }
        Write-LauncherLog "watchdog_restart_claim_failed marker=$RestartRequestPath error=$($_.Exception.Message)"
        throw
    }
}

function Get-CodexRuntimePythonExecutable {
    return Resolve-CodexRuntimePythonExecutable -RepoRoot $ScriptDir
}

function Restore-RestartRequest {
    param([string]$ClaimPath)

    if ([string]::IsNullOrWhiteSpace($ClaimPath) -or -not (Test-Path -LiteralPath $ClaimPath)) {
        return
    }
    if (Test-Path -LiteralPath $RestartRequestPath) {
        Write-LauncherLog "watchdog_restart_restore_deferred marker=$RestartRequestPath claim=$ClaimPath"
        return
    }
    try {
        Move-Item `
            -LiteralPath $ClaimPath `
            -Destination $RestartRequestPath `
            -ErrorAction Stop
        Write-LauncherLog "watchdog_restart_restored marker=$RestartRequestPath claim=$ClaimPath"
    } catch {
        if (Test-Path -LiteralPath $RestartRequestPath) {
            Write-LauncherLog "watchdog_restart_restore_raced marker=$RestartRequestPath claim=$ClaimPath"
            return
        }
        throw
    }
}

function Restore-OrphanedRestartClaims {
    foreach ($claim in @(Get-ChildItem -Path $RestartClaimPattern -File -ErrorAction SilentlyContinue)) {
        $ownerAlive = Test-WatchdogClaimOwnerAlive -ClaimName $claim.Name
        if (-not $ownerAlive) {
            if (Test-RestartClaimMatchesCurrentBot `
                -ClaimPath $claim.FullName `
                -BotScript $BotScript `
                -RuntimeLockPath $RuntimeLockPath) {
                Restore-RestartRequest -ClaimPath $claim.FullName
            } else {
                Remove-Item -LiteralPath $claim.FullName -Force -ErrorAction SilentlyContinue
                Write-LauncherLog "watchdog_restart_orphan_stale claim=$($claim.FullName)"
            }
        }
    }
}
