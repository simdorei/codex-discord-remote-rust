[CmdletBinding()]
param(
    [switch]$DryRun,
    [int]$GraceSeconds = 3,
    [string]$LogPath
)

$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
if ([string]::IsNullOrWhiteSpace($LogPath)) {
    $LogPath = Join-Path $ScriptDir 'resource_cleanup.log'
}

function Write-CleanupLog {
    param([string]$Message)

    $timestamp = (Get-Date).ToString('s')
    Add-Content -LiteralPath $LogPath -Encoding UTF8 -Value "[$timestamp] $Message"
}

function Get-InteractiveSessionIds {
    $sessions = @(Get-Process explorer -ErrorAction SilentlyContinue | Select-Object -ExpandProperty SessionId -Unique)
    if ($sessions.Count -eq 0) {
        throw 'No interactive Explorer session found; refusing to clean up user apps.'
    }
    return $sessions
}

function Test-CleanupCandidate {
    param($Process)

    $name = [string]$Process.Name
    $commandLine = [string]$Process.CommandLine

    if ($Process.ProcessId -eq $PID) {
        return $false
    }

    if ($name -eq 'chrome.exe') {
        return $true
    }
    if ($name -eq 'extension-host.exe') {
        return $true
    }
    if ($name -eq 'cmd.exe' -and $commandLine -like '*extension-host.exe*') {
        return $true
    }
    if ($name -in @('py.exe', 'python.exe', 'pythonw.exe') -and $commandLine -like '*wehago-capture-server.py*') {
        return $true
    }
    if ($name -in @(
        'AdobeCollabSync.exe',
        'CalculatorApp.exe',
        'Smart Wememo.exe',
        'SSScheduler.exe',
        'SDXHelper.exe'
    )) {
        return $true
    }

    return $false
}

function Get-DescendantProcessIds {
    param(
        [int[]]$RootIds,
        [object[]]$Processes
    )

    $childrenByParent = @{}
    foreach ($process in $Processes) {
        $parentId = [int]$process.ParentProcessId
        if (-not $childrenByParent.ContainsKey($parentId)) {
            $childrenByParent[$parentId] = @()
        }
        $childrenByParent[$parentId] += [int]$process.ProcessId
    }

    $seen = @{}
    $queue = [Collections.Generic.Queue[int]]::new()
    foreach ($rootId in $RootIds) {
        $queue.Enqueue($rootId)
    }

    while ($queue.Count -gt 0) {
        $id = $queue.Dequeue()
        if ($seen.ContainsKey($id)) {
            continue
        }
        $seen[$id] = $true
        if ($childrenByParent.ContainsKey($id)) {
            foreach ($childId in $childrenByParent[$id]) {
                $queue.Enqueue([int]$childId)
            }
        }
    }

    return $seen
}

function Get-OpenAIBrowserConnectorProcessIds {
    param([object[]]$Processes)

    $byPid = @{}
    foreach ($process in $Processes) {
        $byPid[[int]$process.ProcessId] = $process
    }

    $rootIds = @()
    foreach ($process in $Processes) {
        $commandLine = [string]$process.CommandLine
        if (
            $commandLine -notlike '*\.codex\plugins\cache\openai-bundled\chrome\*' -and
            $commandLine -notlike '*chrome-extension://hehggadaopoacecdllhhajmbjkdcmajg*'
        ) {
            continue
        }

        $current = $process
        $rootId = [int]$current.ProcessId
        while ($byPid.ContainsKey([int]$current.ParentProcessId)) {
            $parent = $byPid[[int]$current.ParentProcessId]
            if ([string]$parent.Name -notin @('chrome.exe', 'cmd.exe', 'extension-host.exe')) {
                break
            }
            $rootId = [int]$parent.ProcessId
            $current = $parent
        }
        $rootIds += $rootId
    }

    if ($rootIds.Count -eq 0) {
        return @{}
    }
    return Get-DescendantProcessIds -RootIds $rootIds -Processes $Processes
}

function Get-CleanupCandidates {
    $sessionIds = @(Get-InteractiveSessionIds)
    $processes = @(Get-CimInstance Win32_Process | Where-Object {
        $sessionIds -contains $_.SessionId
    })
    $openAIBrowserConnectorIds = Get-OpenAIBrowserConnectorProcessIds -Processes $processes
    @($processes | Where-Object {
        -not $openAIBrowserConnectorIds.ContainsKey([int]$_.ProcessId) -and
        (Test-CleanupCandidate $_)
    } | Sort-Object ProcessId -Unique)
}

$targets = @(Get-CleanupCandidates)
Write-CleanupLog "cleanup_start dry_run=$DryRun target_count=$($targets.Count)"

if ($targets.Count -eq 0) {
    Write-Output 'cleanup_targets: none'
    Write-CleanupLog 'cleanup_done stopped=0 failed=0'
    exit 0
}

Write-Output 'cleanup_targets:'
foreach ($target in $targets) {
    Write-Output "  PID=$($target.ProcessId) Name=$($target.Name) Cmd=$([string]$target.CommandLine)"
}

if ($DryRun) {
    Write-CleanupLog "cleanup_dry_run target_count=$($targets.Count)"
    exit 0
}

$targetIds = @($targets | Select-Object -ExpandProperty ProcessId)
foreach ($id in $targetIds) {
    $process = Get-Process -Id $id -ErrorAction SilentlyContinue
    if ($null -ne $process -and $process.MainWindowHandle -ne 0) {
        try {
            $null = $process.CloseMainWindow()
            Write-Output "close_requested PID=$id Name=$($process.ProcessName)"
            Write-CleanupLog "close_requested pid=$id name=$($process.ProcessName)"
        } catch {
            Write-Output "close_request_failed PID=$id ERROR=$($_.Exception.Message)"
            Write-CleanupLog "close_request_failed pid=$id error=$($_.Exception.Message)"
        }
    }
}

if ($GraceSeconds -gt 0) {
    Start-Sleep -Seconds $GraceSeconds
}

$stopped = 0
$failed = 0
foreach ($id in $targetIds) {
    $process = Get-Process -Id $id -ErrorAction SilentlyContinue
    if ($null -eq $process) {
        Write-Output "already_exited PID=$id"
        Write-CleanupLog "already_exited pid=$id"
        continue
    }

    try {
        Stop-Process -Id $id -Force -ErrorAction Stop
        $stopped += 1
        Write-Output "stopped PID=$id Name=$($process.ProcessName)"
        Write-CleanupLog "stopped pid=$id name=$($process.ProcessName)"
    } catch {
        $failed += 1
        Write-Output "stop_failed PID=$id Name=$($process.ProcessName) ERROR=$($_.Exception.Message)"
        Write-CleanupLog "stop_failed pid=$id name=$($process.ProcessName) error=$($_.Exception.Message)"
    }
}

Start-Sleep -Seconds 1
$remaining = @(foreach ($id in $targetIds) {
    $process = Get-Process -Id $id -ErrorAction SilentlyContinue
    if ($null -ne $process) {
        $process
    }
})

Write-Output 'remaining_targets:'
foreach ($process in $remaining) {
    Write-Output "  PID=$($process.Id) Name=$($process.ProcessName)"
}

Write-CleanupLog "cleanup_done stopped=$stopped failed=$failed remaining=$($remaining.Count)"
if ($failed -gt 0 -or $remaining.Count -gt 0) {
    exit 1
}
exit 0
