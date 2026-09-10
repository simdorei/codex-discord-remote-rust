function Get-MemoryAbVerifiedProcess {
    param(
        [Parameter(Mandatory = $true)][int]$ProcessId,
        [Parameter(Mandatory = $true)][DateTimeOffset]$ExpectedStartUtc,
        [Parameter(Mandatory = $true)][string]$ExpectedExecutablePath,
        [Parameter(Mandatory = $true)][string]$ProcessKind
    )
    $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if ($null -eq $process) {
        throw "$ProcessKind process identity was lost: PID $ProcessId is not running."
    }
    try {
        $actualStartUtc = $process.StartTime.ToUniversalTime()
        if ($actualStartUtc.Ticks -ne $ExpectedStartUtc.UtcDateTime.Ticks) {
            throw (
                "$ProcessKind process identity mismatch: PID $ProcessId expected start " +
                "$($ExpectedStartUtc.UtcDateTime.ToString('o')) but found " +
                "$($actualStartUtc.ToString('o'))."
            )
        }
        $actualExecutablePath = [IO.Path]::GetFullPath([string]$process.Path)
        if (-not $actualExecutablePath.Equals(
            $ExpectedExecutablePath,
            [StringComparison]::OrdinalIgnoreCase
        )) {
            throw (
                "$ProcessKind executable identity mismatch: PID $ProcessId expected " +
                "'$ExpectedExecutablePath' but found '$actualExecutablePath'."
            )
        }
        return $process
    } catch {
        $process.Dispose()
        throw
    }
}

function Assert-MemoryAbAppServerParent {
    param(
        [Parameter(Mandatory = $true)][int]$ProcessId,
        [Parameter(Mandatory = $true)][int]$ExpectedParentPid
    )
    try {
        $rows = @(Get-CimInstance -ClassName Win32_Process `
            -Filter "ProcessId = $ProcessId" -ErrorAction Stop)
    } catch {
        throw "app-server parent verification failed for PID ${ProcessId}: $($_.Exception.Message)"
    }
    if ($rows.Count -ne 1) {
        throw "app-server parent verification failed: PID $ProcessId was not found."
    }
    $actualParentPid = [int]$rows[0].ParentProcessId
    if ($actualParentPid -ne $ExpectedParentPid) {
        throw (
            "app-server parent mismatch: PID $ProcessId has parent $actualParentPid; " +
            "expected bot PID $ExpectedParentPid."
        )
    }
}

function Assert-MemoryAbNoSecondLiveDiscordBot {
    param([Parameter(Mandatory = $true)][int]$ExpectedBotPid)
    $query = @"
SELECT ProcessId, Name, CommandLine FROM Win32_Process
WHERE Name = 'cdr-runtime.exe' OR Name = 'python.exe' OR Name = 'pythonw.exe'
"@
    try {
        $rows = @(Get-CimInstance -Query $query -ErrorAction Stop)
    } catch {
        throw "duplicate Discord bot verification failed: $($_.Exception.Message)"
    }
    foreach ($row in $rows) {
        $candidatePid = [int]$row.ProcessId
        if ($candidatePid -eq $ExpectedBotPid) { continue }
        $name = [string]$row.Name
        if ($name.Equals('cdr-runtime.exe', [StringComparison]::OrdinalIgnoreCase)) {
            throw "second live Discord bot detected: cdr-runtime.exe PID $candidatePid."
        }
        $commandLine = [string]$row.CommandLine
        if ([string]::IsNullOrWhiteSpace($commandLine)) {
            throw "duplicate Discord bot verification could not read python PID $candidatePid."
        }
        if ($commandLine -match '(?i)codex_discord_bot\.py') {
            throw "second live Discord bot detected: python PID $candidatePid."
        }
    }
}

function Get-MemoryAbProcessSnapshot {
    param([Parameter(Mandatory = $true)][Diagnostics.Process]$Process)
    [void]$Process.Refresh()
    try {
        return [pscustomobject]@{
            WorkingSetBytes = [long]$Process.WorkingSet64
            PrivateMemoryBytes = [long]$Process.PrivateMemorySize64
            CpuTotalSeconds = [double]$Process.TotalProcessorTime.TotalSeconds
            HandleCount = [int]$Process.HandleCount
            ThreadCount = [int]$Process.Threads.Count
        }
    } catch {
        throw "process resource sampling failed for PID $($Process.Id): $($_.Exception.Message)"
    }
}

function Get-MemoryAbCpuOneCorePercent {
    param(
        [Parameter(Mandatory = $true)][double]$CurrentTotalSeconds,
        [AllowNull()]$PreviousTotalSeconds,
        [Parameter(Mandatory = $true)][double]$ElapsedSeconds
    )
    if ($null -eq $PreviousTotalSeconds -or $ElapsedSeconds -le 0) { return 0.0 }
    $cpuDelta = $CurrentTotalSeconds - [double]$PreviousTotalSeconds
    if ($cpuDelta -lt 0) {
        throw 'process CPU time moved backwards while the PID identity remained bound.'
    }
    return [math]::Round(($cpuDelta / $ElapsedSeconds) * 100.0, 6)
}
