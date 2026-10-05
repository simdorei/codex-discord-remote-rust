# Full tool-host recovery reuses the existing exact-identity bot restart.
# Read-only planning must succeed before request cancellation or process mutation.
function Get-CdrToolsRecoveryPlan([string]$RepoRoot,[string]$CodexHome,[string]$ThreadId) {
    $lockPath=Get-CdrWriterLockPath $CodexHome $ThreadId
    $package=Get-AppxPackage -Name OpenAI.Codex | Select-Object -First 1
    if ($null -eq $package) { throw 'Installed Codex desktop package was not found.' }
    $desktopPath=Join-Path $package.InstallLocation 'app\ChatGPT.exe'
    if (-not [IO.File]::Exists($desktopPath)) { throw 'Installed desktop executable is missing.' }
    $runtimePath=Join-Path $RepoRoot 'target\release\cdr-runtime.exe'
    $runtimeLock=[IO.File]::ReadAllText((Join-Path $RepoRoot '.codex_discord_rust.runtime.lock'))
    if ($runtimeLock -notmatch '(?m)^pid=(\d+)\r?$') { throw 'Bot process identity is missing.' }
    $bot=Get-Process -Id ([int]$Matches[1]) -ErrorAction Stop
    $desktop=$null
    try {
        [void]$bot.Handle
        if ($bot.Path -ine $runtimePath) { throw 'Bot process path changed; recovery refused.' }
        $ticks=$bot.StartTime.ToUniversalTime().Ticks; $ticks-=($ticks%10)
        $botIdentity="$($bot.Id)|$ticks"
        if ($runtimeLock -notmatch '(?m)^started_at=(\d+)\r?$' -or
            [math]::Abs(([DateTimeOffset]$bot.StartTime.ToUniversalTime()).ToUnixTimeSeconds()-[long]$Matches[1]) -gt 2) {
            throw 'Bot lock does not match its creation time.'
        }
        $entries=@(Get-CimInstance Win32_Process -Filter "Name='ChatGPT.exe'" |
            Where-Object { $_.ExecutablePath -ieq $desktopPath })
        $ids=@($entries | ForEach-Object { $_.ProcessId })
        $roots=@($entries | Where-Object { $_.ParentProcessId -notin $ids })
        if ($roots.Count -gt 1) { throw 'Multiple desktop roots found; recovery refused.' }
        $desktopIdentity='stopped'
        if ($roots.Count -eq 1) {
            $desktop=Get-Process -Id ([int]$roots[0].ProcessId) -ErrorAction Stop
            [void]$desktop.Handle
            if ($desktop.Path -ine $desktopPath -or
                [math]::Abs(($desktop.StartTime.ToUniversalTime()-$roots[0].CreationDate.ToUniversalTime()).Ticks) -gt 10) {
                throw 'Desktop process identity changed.'
            }
            $desktopIdentity=Get-CdrRecoveryIdentity $desktop
        }
        $owners=@(Get-CdrWriterOwners $lockPath)
        if ($owners.Count -gt 1) { throw 'Writer ownership is ambiguous.' }
        $writerIdentity='none'
        if ($owners.Count -eq 1) {
            $writer=Get-Process -Id ([int]$owners[0].Process.Pid) -ErrorAction Stop
            try {
                [void]$writer.Handle
                $fileTime=([long]$owners[0].Process.Started.dwHighDateTime -shl 32) -bor
                    ([long]$owners[0].Process.Started.dwLowDateTime -band 0xffffffffL)
                $entry=Get-CimInstance Win32_Process -Filter "ProcessId=$($writer.Id)"
                $allowed=@($bot.Id)
                if ($null -ne $desktop) { $allowed+=@($desktop.Id) }
                if ($writer.StartTime.ToUniversalTime().ToFileTimeUtc() -ne $fileTime -or
                    [IO.Path]::GetFileName($writer.Path) -ine 'codex.exe' -or $null -eq $entry -or
                    $entry.ParentProcessId -notin $allowed -or
                    $entry.CommandLine -notmatch '(?:^|\s)app-server(?:\s|$)') {
                    throw 'Writer belongs to an unverified client; nothing was stopped.'
                }
                $writerIdentity=Get-CdrRecoveryIdentity $writer
            } finally { $writer.Dispose() }
        }
        return [pscustomobject]@{Version=1;State='tools_recovery';ThreadId=$ThreadId;RepoRoot=$RepoRoot
            CodexHome=[IO.Path]::GetFullPath($CodexHome);LockPath=$lockPath
            DesktopIdentity=$desktopIdentity;DesktopPath=$desktopPath;BotIdentity=$botIdentity
            WriterIdentity=$writerIdentity;RecoveryIdentity=($desktopIdentity+';'+$botIdentity+';'+$writerIdentity)}
    } finally { $bot.Dispose(); if ($null -ne $desktop) { $desktop.Dispose() } }
}

function Assert-CdrSameToolsHosts($Expected,$Current) {
    foreach ($field in @('ThreadId','RecoveryIdentity','DesktopIdentity','BotIdentity','WriterIdentity','DesktopPath','RepoRoot','CodexHome')) {
        if ($Expected.$field -cne $Current.$field) { throw "Recovery host identity changed: $field" }
    }
    if ($Current.State -cne 'tools_recovery') { throw 'Invalid tool recovery plan.' }
}

function Invoke-CdrToolsRestart($Plan,[string]$ReceiptPath) {
    Assert-CdrSameToolsHosts $Plan (Get-CdrToolsRecoveryPlan $Plan.RepoRoot $Plan.CodexHome $Plan.ThreadId)
    $desktop=$null; $owned=$null; $replacement=$null; $botWorker=$null; $stopped=$false
    $botResult=$null; $botError=''
    try {
        if ($Plan.DesktopIdentity -cne 'stopped') {
            $desktop=Get-Process -Id ([int]$Plan.DesktopIdentity.Split('|')[0]) -ErrorAction Stop
            [void]$desktop.Handle
            if ((Get-CdrRecoveryIdentity $desktop) -cne $Plan.DesktopIdentity) { throw 'Desktop identity changed.' }
            $owned=Get-CdrOwnedProcessHandles $desktop
            if (@($owned | Where-Object { $_.Id -eq [int]$Plan.BotIdentity.Split('|')[0] }).Count -ne 0) {
                throw 'Bot is inside the desktop tree; no overlapping process stop was attempted.'
            }
        }
        Assert-CdrSameToolsHosts $Plan (Get-CdrToolsRecoveryPlan $Plan.RepoRoot $Plan.CodexHome $Plan.ThreadId)
        Save-CdrDesktopRecoveryReceipt $ReceiptPath @{Phase='stopping';ThreadId=$Plan.ThreadId;Scope='desktop_and_bot_tools'}
        if ($null -ne $desktop) {
            [void]$desktop.CloseMainWindow()
            [void]$desktop.WaitForExit(2000)
            foreach ($p in $owned) {
                if (-not $p.HasExited) { try { $p.Kill() } catch { if (-not $p.HasExited) { throw } } }
            }
            foreach ($p in $owned) { if (-not $p.WaitForExit(5000)) { throw 'Desktop descendant did not exit.' } }
        }
        $stopped=$true
        try {
            Save-CdrDesktopRecoveryReceipt $ReceiptPath @{Phase='restarting_bot';ThreadId=$Plan.ThreadId;Scope='desktop_and_bot_tools'}
            $shell=Join-Path ([Environment]::GetFolderPath('System')) 'WindowsPowerShell\v1.0\powershell.exe'
            $entry=Join-Path $Plan.RepoRoot 'codex-discord-rust-restart.ps1'
            $botWorker=Start-Process -FilePath $shell -WindowStyle Hidden -PassThru -ArgumentList @(
                '-NoProfile','-ExecutionPolicy','Bypass','-File',('"'+$entry+'"'),
                '-RepoRoot',('"'+$Plan.RepoRoot+'"'),'-Force','-ForceWorker',
                '-ExpectedBotIdentity',('"'+$Plan.BotIdentity+'"'))
            if (-not $botWorker.WaitForExit(120000)) { throw 'Bot restart is still unverified; inspect its force-restart journal before retrying.' }
            if ($botWorker.ExitCode -ne 0) { throw 'Bot restart controller failed; inspect codex-discord-rust-restart.log.' }
            $botResult=[IO.File]::ReadAllText((Join-Path $Plan.RepoRoot '.codex_discord_rust.force.completed')) | ConvertFrom-Json
            $replacementIdentity=$botResult.ReplacementIdentity
            $replacementParts=@()
            if ($replacementIdentity -is [string] -and $replacementIdentity -cmatch '\A[1-9][0-9]*\|[1-9][0-9]*\z') {
                $replacementParts=$replacementIdentity.Split('|')
            }
            [uint32]$replacementPid=0; [long]$replacementTicks=0
            if ($botResult.InterruptedIdentity -isnot [string] -or
                $botResult.InterruptedIdentity -cne $Plan.BotIdentity -or
                $replacementParts.Count -ne 2 -or
                -not [uint32]::TryParse($replacementParts[0],[ref]$replacementPid) -or
                -not [long]::TryParse($replacementParts[1],[ref]$replacementTicks) -or
                $replacementTicks -gt [DateTime]::MaxValue.Ticks -or
                $replacementIdentity -ceq $Plan.BotIdentity) { throw 'Bot restart receipt does not match this recovery.' }
        } catch { $botError=$_.Exception.Message }
        # Restore the desktop even if the independently recorded bot restart failed.
        Save-CdrDesktopRecoveryReceipt $ReceiptPath @{Phase='restarting_desktop';ThreadId=$Plan.ThreadId;BotError=$botError}
        $replacement=Start-Process -FilePath $Plan.DesktopPath -WindowStyle Hidden -PassThru
        $deadline=[DateTime]::UtcNow.AddSeconds(35);$observed=@()
        do {
            $replacement.Refresh()
            if ($replacement.HasExited) { throw 'Replacement desktop exited during startup.' }
            $observed=@(Get-CimInstance Win32_Process -Filter "ParentProcessId=$($replacement.Id) AND Name='codex.exe'")
            if ($observed.Count -gt 0) { break }
            Start-Sleep -Milliseconds 500
        } while ([DateTime]::UtcNow -lt $deadline)
        if ($observed.Count -eq 0) { throw 'Desktop started, but a replacement Codex backend was not observed.' }
        $released=@(Get-CdrWriterOwners $Plan.LockPath).Count -eq 0
        # A replacement bot may legitimately acquire the thread again. Full
        # recovery proves host replacement, not permanent absence of a writer.
        $phase=if ($botError) {'failed'} else {'restarted'}
        Save-CdrDesktopRecoveryReceipt $ReceiptPath @{
            Phase=$phase;ThreadId=$Plan.ThreadId;Scope='desktop_and_bot_tools';WriterReleased=$released
            PreviousDesktopIdentity=$Plan.DesktopIdentity;DesktopIdentity=(Get-CdrRecoveryIdentity $replacement)
            PreviousBotIdentity=$Plan.BotIdentity;BotIdentity=$botResult.ReplacementIdentity
            BackendObserved=$true;ToolProbeVerified=$false;Error=$botError;FinishedAt=[DateTime]::UtcNow.ToString('o')}
    } finally {
        if ($null -ne $owned) { foreach ($p in $owned) { $p.Dispose() } }
        elseif ($null -ne $desktop) { $desktop.Dispose() }
        if ($null -ne $replacement) { $replacement.Dispose() }
        if ($null -ne $botWorker) { $botWorker.Dispose() }
    }
}
