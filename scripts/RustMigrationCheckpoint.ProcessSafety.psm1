Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.Common.psm1') -ErrorAction Stop
Set-StrictMode -Version Latest

if (-not ('CodexDiscordRemote.CheckpointNativePaths' -as [type])) {
    $null = Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
namespace CodexDiscordRemote {
    public static class CheckpointNativePaths {
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern uint GetShortPathName(
            string longPath, StringBuilder shortPath, uint bufferLength);
    }
}
'@ -ErrorAction Stop
}

function Get-CdrWindowsShortPath([string]$Path) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    $capacity = 32768
    $buffer = [Text.StringBuilder]::new($capacity)
    $written = [CodexDiscordRemote.CheckpointNativePaths]::GetShortPathName(
        $fullPath, $buffer, [uint32]$capacity
    )
    if ($written -eq 0 -or $written -ge $capacity) {
        $code = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        throw "Cannot resolve legacy Python bot short path: win32=$code"
    }
    return $buffer.ToString()
}

function Get-CdrCommandLineTokens([string]$CommandLine) {
    $tokens = [Collections.Generic.List[string]]::new()
    foreach ($match in [regex]::Matches($CommandLine, '"[^"]*"|[^\s"]+')) {
        $value = [string]$match.Value
        if ($value.Length -ge 2 -and $value[0] -ceq '"' -and
            $value[$value.Length - 1] -ceq '"') {
            $value = $value.Substring(1, $value.Length - 2)
        }
        $tokens.Add($value)
    }
    return [string[]]$tokens.ToArray()
}

function Test-CdrLegacyPythonBotCommandLine(
    [string]$CommandLine,
    [string]$AbsoluteScriptPath,
    [string]$ShortScriptPath
) {
    $tokens = @(Get-CdrCommandLineTokens $CommandLine)
    $absoluteNeedle = $AbsoluteScriptPath.ToLowerInvariant().Replace('/', '\')
    $shortNeedle = $ShortScriptPath.ToLowerInvariant().Replace('/', '\')
    $shortSeparator = [Math]::Max($shortNeedle.LastIndexOf('\'), $shortNeedle.LastIndexOf(':'))
    $shortBasename = $shortNeedle.Substring($shortSeparator + 1).TrimEnd([char[]]@('.', ' '))
    for ($index = 0; $index -lt $tokens.Count; $index++) {
        $token = ([string]$tokens[$index]).ToLowerInvariant().Replace('/', '\')
        $separator = [Math]::Max($token.LastIndexOf('\'), $token.LastIndexOf(':'))
        $basename = if ($separator -ge 0) {
            $token.Substring($separator + 1)
        } else { $token }
        $windowsBasename = $basename.TrimEnd([char[]]@('.', ' '))
        if ($token -ceq $absoluteNeedle -or $token -ceq $shortNeedle -or
            $windowsBasename -ceq 'codex_discord_bot.py' -or
            $windowsBasename -ceq $shortBasename) {
            return $true
        }
        $isSplitModuleSwitch = $token -match '^-[a-z?]*m$'
        if ($isSplitModuleSwitch -and $index + 1 -lt $tokens.Count -and
            ([string]$tokens[$index + 1]).ToLowerInvariant() -ceq 'codex_discord_bot') {
            return $true
        }
        if ($token -match '^-[a-z?]*mcodex_discord_bot$') { return $true }
    }
    return $false
}

function Get-CdrLegacyPythonBotProcessSnapshot {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [scriptblock]$ProcessRowsQuery,
        [scriptblock]$ShortPathQuery
    )
    try {
        $rows = if ($null -eq $ProcessRowsQuery) {
            @(Get-CimInstance -ClassName Win32_Process -ErrorAction Stop)
        } else { @(& $ProcessRowsQuery) }
    } catch { throw "Cannot enumerate legacy Python bot processes: $($_.Exception.Message)" }
    $scriptPath = [IO.Path]::GetFullPath(
        (Join-Path $RepoRoot 'codex_discord_bot.py')
    )
    try {
        $shortPath = if ($null -eq $ShortPathQuery) {
            if ([IO.File]::Exists($scriptPath)) { Get-CdrWindowsShortPath $scriptPath }
            else { $scriptPath } # Retired source need not exist to detect a stray legacy writer.
        } else { & $ShortPathQuery $scriptPath }
        if ([string]::IsNullOrWhiteSpace([string]$shortPath)) {
            throw 'short path query returned an empty value'
        }
        $shortPath = [string]$shortPath
    } catch { throw "Cannot verify legacy Python bot short path: $($_.Exception.Message)" }
    $identities = [Collections.Generic.List[string]]::new()
    foreach ($row in $rows) {
        $name = ([string]$row.Name).ToLowerInvariant()
        if ($name -notmatch '^(?:pyw?|python(?:\d+(?:\.\d+)?)?w?)\.exe$') { continue }
        $commandLine = [string]$row.CommandLine
        if ([string]::IsNullOrWhiteSpace($commandLine)) {
            throw "Cannot verify Python process command line: pid=$($row.ProcessId)"
        }
        if (-not (Test-CdrLegacyPythonBotCommandLine $commandLine $scriptPath $shortPath)) {
            continue
        }
        try {
            $processId = [uint32]$row.ProcessId
            if ($processId -eq 0) { throw 'PID is zero' }
            $ticks = ([datetime]$row.CreationDate).ToUniversalTime().Ticks
            $identities.Add("python-bot|$processId|$ticks")
        } catch {
            throw "Cannot verify legacy Python bot identity: $($_.Exception.Message)"
        }
    }
    return [string[]]$identities.ToArray()
}

function Get-CdrCheckpointForbiddenProcessSnapshot {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [scriptblock]$NativeSnapshotQuery,
        [scriptblock]$ProcessRowsQuery,
        [scriptblock]$ShortPathQuery
    )
    $native = if ($null -eq $NativeSnapshotQuery) {
        @(Get-CdrCheckpointNativeProcessSnapshot -RepoRoot $RepoRoot)
    } else { @(& $NativeSnapshotQuery) }
    $python = @(Get-CdrLegacyPythonBotProcessSnapshot `
        -RepoRoot $RepoRoot -ProcessRowsQuery $ProcessRowsQuery `
        -ShortPathQuery $ShortPathQuery)
    return [string[]]@($native + $python | Sort-Object -Unique)
}

function Get-CdrCheckpointNativeProcessSnapshot {
    param([string]$RepoRoot, [scriptblock]$ProcessQuery)
    $root = [IO.Path]::GetFullPath($RepoRoot).TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    $ownedPids = foreach ($name in @('.codex_discord_rust.runtime.lock', '.codex_discord_bot.runtime.lock')) {
        $lockPath = Join-Path $RepoRoot $name
        if (-not [IO.File]::Exists($lockPath)) { continue }
        $rows = @([IO.File]::ReadAllLines($lockPath) | Where-Object { $_ -match '^pid=' })
        if ($rows.Count -ne 1 -or $rows[0] -notmatch '^pid=([1-9][0-9]*)$') {
            throw "Cannot verify runtime lock identity: $name"
        }
        [long]$Matches[1]
    }
    $identities = foreach ($name in @('cdr-runtime', 'cdr-offline-soak', 'cdr-mcp-server', 'cdr-pro-helper')) {
        foreach ($process in @(Get-CdrForbiddenProcessCandidates -Name $name -ProcessQuery $ProcessQuery)) {
            try {
                $path = [string]$process.Path
                if ([string]::IsNullOrWhiteSpace($path)) {
                    throw "Cannot verify executable path for checkpoint process: name=$name pid=$($process.Id)"
                }
                $path = [IO.Path]::GetFullPath($path)
                if ($ownedPids -contains [long]$process.Id -or
                    $path.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) {
                    Get-CdrForbiddenProcessIdentity $process $name
                }
            } finally { if ($process -is [IDisposable]) { $process.Dispose() } }
        }
    }
    return @($identities | Sort-Object -Unique)
}

function Assert-CdrCheckpointBotOff([object[]]$Snapshot, [string]$Phase) {
    $count = if ($null -eq $Snapshot) { 0 } else { $Snapshot.Length }
    if ($count -ne 0) {
        throw "Checkpoint requires zero bot/runtime processes: phase=$Phase count=$count"
    }
}

Export-ModuleMember -Function @(
    'Get-CdrWindowsShortPath', 'Test-CdrLegacyPythonBotCommandLine',
    'Get-CdrLegacyPythonBotProcessSnapshot',
    'Get-CdrCheckpointForbiddenProcessSnapshot',
    'Get-CdrCheckpointNativeProcessSnapshot',
    'Assert-CdrCheckpointBotOff'
)
