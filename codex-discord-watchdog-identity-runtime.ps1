function Get-CodexBotProcessIdentityById {
    param(
        [string]$BotScript,
        [int]$ProcessId
    )

    try {
        $process = Get-CimInstance Win32_Process `
            -Filter "ProcessId=$ProcessId" `
            -ErrorAction Stop
        if ($process -eq $null) {
            return ""
        }
        $name = ([string]$process.Name).ToLowerInvariant()
        if ($name -notin @('py.exe', 'python.exe', 'pythonw.exe')) {
            return ""
        }
        $needle = [IO.Path]::GetFullPath($BotScript).ToLowerInvariant()
        $commandLine = ([string]$process.CommandLine).ToLowerInvariant()
        if (-not $commandLine -or -not $commandLine.Contains($needle)) {
            return ""
        }
        $startedAt = ([datetime]$process.CreationDate).ToUniversalTime()
        $normalizedTicks = $startedAt.Ticks - ($startedAt.Ticks % 10)
        return "$ProcessId|$normalizedTicks"
    } catch {
        return ""
    }
}

function Get-CodexBotProcessIdentity {
    param(
        [string]$BotScript,
        [string]$RuntimeLockPath
    )

    if (-not (Test-Path -LiteralPath $RuntimeLockPath)) {
        return ""
    }
    try {
        $processIdText = [System.IO.File]::ReadAllText($RuntimeLockPath).Trim()
        if ($processIdText -notmatch '^\d+$') {
            return ""
        }
        return Get-CodexBotProcessIdentityById `
            -BotScript $BotScript `
            -ProcessId ([int]$processIdText)
    } catch {
        return ""
    }
}

function Get-BoundProcessIdentityFromMarker {
    param([string]$MarkerPath)

    if (-not (Test-Path -LiteralPath $MarkerPath)) {
        return ""
    }
    try {
        $markerText = [System.IO.File]::ReadAllText($MarkerPath).Trim()
    } catch {
        return ""
    }
    if (-not $markerText.StartsWith('identity=', [StringComparison]::Ordinal)) {
        return ""
    }
    $identity = $markerText.Substring('identity='.Length)
    if ($identity -notmatch '^\d+\|\d+$') {
        return ""
    }
    return $identity
}

function Test-RestartClaimMatchesCurrentBot {
    param(
        [string]$ClaimPath,
        [string]$BotScript,
        [string]$RuntimeLockPath
    )

    if (-not (Test-Path -LiteralPath $ClaimPath)) {
        return $false
    }
    $expectedIdentity = Get-BoundProcessIdentityFromMarker -MarkerPath $ClaimPath
    if (-not $expectedIdentity) {
        return $false
    }
    $currentIdentity = Get-CodexBotProcessIdentity `
        -BotScript $BotScript `
        -RuntimeLockPath $RuntimeLockPath
    return (
        $expectedIdentity -and
        [string]::Equals(
            $expectedIdentity,
            $currentIdentity,
            [StringComparison]::Ordinal
        )
    )
}

function Test-WatchdogClaimOwnerAlive {
    param([string]$ClaimName)

    if ($ClaimName -notmatch '\.claimed\.(\d+)\.(\d+)$') {
        return $false
    }
    [int]$ownerPid = 0
    [long]$ownerStartedTicks = 0
    if (
        -not [int]::TryParse($Matches[1], [ref]$ownerPid) -or
        -not [long]::TryParse($Matches[2], [ref]$ownerStartedTicks)
    ) {
        return $false
    }
    $owner = Get-Process -Id $ownerPid -ErrorAction SilentlyContinue
    if ($owner -eq $null) {
        return $false
    }
    try {
        $currentTicks = $owner.StartTime.ToUniversalTime().Ticks
        $currentTicks -= ($currentTicks % 10)
        return $currentTicks -eq $ownerStartedTicks
    } catch {
        return $false
    } finally {
        $owner.Dispose()
    }
}
