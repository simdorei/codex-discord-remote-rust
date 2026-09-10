function Restore-StopRequest {
    param([string]$ClaimPath)

    if ([string]::IsNullOrWhiteSpace($ClaimPath) -or -not (Test-Path -LiteralPath $ClaimPath)) {
        return
    }
    if (Test-Path -LiteralPath $StopRequestPath) {
        Write-LauncherLog "watchdog_stop_restore_deferred marker=$StopRequestPath claim=$ClaimPath"
        return
    }
    try {
        Move-Item `
            -LiteralPath $ClaimPath `
            -Destination $StopRequestPath `
            -ErrorAction Stop
        Write-LauncherLog "watchdog_stop_restored marker=$StopRequestPath claim=$ClaimPath"
    } catch {
        if (Test-Path -LiteralPath $StopRequestPath) {
            Write-LauncherLog "watchdog_stop_restore_raced marker=$StopRequestPath claim=$ClaimPath"
            return
        }
        throw
    }
}

function Restore-OrphanedStopClaims {
    foreach ($claim in @(Get-ChildItem -Path $StopClaimPattern -File -ErrorAction SilentlyContinue)) {
        $ownerAlive = Test-WatchdogClaimOwnerAlive -ClaimName $claim.Name
        if ($ownerAlive) {
            continue
        }

        $expectedIdentity = Get-BoundProcessIdentityFromMarker `
            -MarkerPath $claim.FullName
        if (-not $expectedIdentity) {
            Remove-Item -LiteralPath $claim.FullName -Force -ErrorAction SilentlyContinue
            Write-LauncherLog "watchdog_stop_orphan_unbound claim=$($claim.FullName)"
            continue
        }
        $expectedPid = [int]($expectedIdentity -split '\|', 2)[0]
        $currentIdentity = Get-CodexBotProcessIdentityById `
            -BotScript $BotScript `
            -ProcessId $expectedPid
        if ($currentIdentity -ne $expectedIdentity) {
            Remove-Item -LiteralPath $claim.FullName -Force -ErrorAction SilentlyContinue
            Write-LauncherLog "watchdog_stop_orphan_stale claim=$($claim.FullName)"
            continue
        }
        Restore-StopRequest -ClaimPath $claim.FullName
    }
}
