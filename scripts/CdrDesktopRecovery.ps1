# Exact-owner desktop recovery. Never delete writer locks or terminate by image name.
function Initialize-CdrWriterInspector {
    if ('CdrWriterInspector' -as [type]) { return }
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
public static class CdrWriterInspector {
    [StructLayout(LayoutKind.Sequential)] public struct Unique {
        public uint Pid; public System.Runtime.InteropServices.ComTypes.FILETIME Started;
    }
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct Info {
        public Unique Process;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=256)] public string Name;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=64)] public string Service;
        public uint Type, Status, Session;
        [MarshalAs(UnmanagedType.Bool)] public bool Restartable;
    }
    [DllImport("rstrtmgr.dll", CharSet=CharSet.Unicode)] static extern int RmStartSession(out uint h, uint flags, string key);
    [DllImport("rstrtmgr.dll", CharSet=CharSet.Unicode)] static extern int RmRegisterResources(uint h, uint n, string[] paths, uint np, IntPtr p, uint ns, IntPtr s);
    [DllImport("rstrtmgr.dll")] static extern int RmGetList(uint h, out uint needed, ref uint count, [In,Out] Info[] list, ref uint reasons);
    [DllImport("rstrtmgr.dll")] static extern int RmEndSession(uint h);
    public static Info[] Owners(string path) {
        uint h; int code=RmStartSession(out h, 0, Guid.NewGuid().ToString("N"));
        if(code!=0) throw new Win32Exception(code);
        try {
            code=RmRegisterResources(h,1,new[]{path},0,IntPtr.Zero,0,IntPtr.Zero);
            if(code!=0) throw new Win32Exception(code);
            for(int retry=0;retry<3;retry++) {
                uint needed=0,count=0,reasons=0;
                code=RmGetList(h,out needed,ref count,null,ref reasons);
                if(code==0) return new Info[0];
                if(code!=234 || needed>1024) throw new Win32Exception(code);
                count=needed; var list=new Info[count];
                code=RmGetList(h,out needed,ref count,list,ref reasons);
                if(code==234) continue;
                if(code!=0) throw new Win32Exception(code);
                Array.Resize(ref list,(int)count); return list;
            }
            throw new InvalidOperationException("Writer owner changed repeatedly; nothing was stopped.");
        } finally { RmEndSession(h); }
    }
}
'@
}

function Get-CdrRecoveryIdentity($Process) {
    $ticks=$Process.StartTime.ToUniversalTime().Ticks
    return "$($Process.Id)|$ticks"
}

function Get-CdrWriterLockPath([string]$CodexHome,[string]$ThreadId) {
    if ($ThreadId -cnotmatch '^[a-f0-9]{8}(-[a-f0-9]{4}){3}-[a-f0-9]{12}$') {
        throw 'Recovery requires an exact thread UUID.'
    }
    return Join-Path ([IO.Path]::GetFullPath($CodexHome)) "thread-writer-locks\$ThreadId.lock"
}

function Get-CdrWriterOwners([string]$LockPath) {
    if (-not [IO.File]::Exists($LockPath)) { return @() }
    Initialize-CdrWriterInspector
    return @([CdrWriterInspector]::Owners($LockPath))
}

function Get-CdrDesktopRecoveryPlan([string]$CodexHome,[string]$ThreadId) {
    $lockPath=Get-CdrWriterLockPath $CodexHome $ThreadId
    $owners=@(Get-CdrWriterOwners $lockPath)
    if ($owners.Count -eq 0) {
        return [pscustomobject]@{ Version=1; ThreadId=$ThreadId; State='unlocked'; LockPath=$lockPath }
    }
    if ($owners.Count -ne 1) { throw 'Writer ownership is not unique; nothing was stopped.' }
    $writer=Get-Process -Id $owners[0].Process.Pid -ErrorAction Stop
    $desktop=$null
    try {
        [void]$writer.Handle
        $fileTime=([long]$owners[0].Process.Started.dwHighDateTime -shl 32) -bor
            ([long]$owners[0].Process.Started.dwLowDateTime -band 0xffffffffL)
        if ($writer.StartTime.ToUniversalTime().ToFileTimeUtc() -ne $fileTime -or
            [IO.Path]::GetFileName($writer.Path) -ine 'codex.exe') {
            throw 'Writer process identity changed or is not Codex; preserved.'
        }
        $entry=Get-CimInstance Win32_Process -Filter "ProcessId=$($writer.Id)"
        if ($null -eq $entry -or
            [math]::Abs(($entry.CreationDate.ToUniversalTime()-$writer.StartTime.ToUniversalTime()).Ticks) -gt 10 -or
            $entry.CommandLine -notmatch '(?:^|\s)app-server(?:\s|$)') {
            throw 'Writer is not a verified Codex app-server; preserved.'
        }
        $desktop=Get-Process -Id $entry.ParentProcessId -ErrorAction Stop
        [void]$desktop.Handle
        $package=Get-AppxPackage -Name OpenAI.Codex | Select-Object -First 1
        if ($null -eq $package -or $desktop.Path -ine (Join-Path $package.InstallLocation 'app\ChatGPT.exe') -or
            $desktop.StartTime.ToUniversalTime() -gt $writer.StartTime.ToUniversalTime()) {
            throw 'Writer belongs to another client (possibly the bot); desktop restart refused.'
        }
        return [pscustomobject]@{
            Version=1; ThreadId=$ThreadId; State='desktop_owned'; CodexHome=[IO.Path]::GetFullPath($CodexHome)
            LockPath=$lockPath; WriterIdentity=(Get-CdrRecoveryIdentity $writer)
            DesktopIdentity=(Get-CdrRecoveryIdentity $desktop); DesktopPath=$desktop.Path
        }
    } finally {
        $writer.Dispose()
        if ($null -ne $desktop) { $desktop.Dispose() }
    }
}

function Save-CdrDesktopRecoveryReceipt([string]$Path,$Receipt) {
    $temporary=$Path+'.'+[guid]::NewGuid().ToString('N')+'.tmp'
    [IO.File]::WriteAllText($temporary,($Receipt | ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $temporary -Destination $Path -Force
}

function Assert-CdrSameDesktopOwner($Expected,$Current) {
    if ($Current.State -cne 'desktop_owned' -or $Expected.ThreadId -cne $Current.ThreadId -or
        $Expected.WriterIdentity -cne $Current.WriterIdentity -or
        $Expected.DesktopIdentity -cne $Current.DesktopIdentity -or
        $Expected.DesktopPath -ine $Current.DesktopPath) {
        throw 'Desktop/writer ownership changed after preparation; no replacement process was stopped.'
    }
}

function Invoke-CdrDesktopRestart($Plan,[string]$ReceiptPath) {
    Assert-CdrSameDesktopOwner $Plan (Get-CdrDesktopRecoveryPlan $Plan.CodexHome $Plan.ThreadId)
    $desktop=Get-Process -Id ([int]($Plan.DesktopIdentity.Split('|')[0])) -ErrorAction Stop
    $owned=$null
    try {
        [void]$desktop.Handle
        if ((Get-CdrRecoveryIdentity $desktop) -cne $Plan.DesktopIdentity) { throw 'Desktop identity changed.' }
        # Existing force-restart helper pins each descendant and refuses its own ancestry.
        $owned=Get-CdrOwnedProcessHandles -RootProcess $desktop
        Assert-CdrSameDesktopOwner $Plan (Get-CdrDesktopRecoveryPlan $Plan.CodexHome $Plan.ThreadId)
        Save-CdrDesktopRecoveryReceipt $ReceiptPath @{
            Phase='stopping'; ThreadId=$Plan.ThreadId; DesktopIdentity=$Plan.DesktopIdentity
        }
        [void]$desktop.CloseMainWindow()
        [void]$desktop.WaitForExit(3000)
        foreach ($process in $owned) {
            if (-not $process.HasExited) {
                try { $process.Kill() } catch { if (-not $process.HasExited) { throw } }
            }
        }
        foreach ($process in $owned) {
            if (-not $process.WaitForExit(5000)) { throw "Desktop descendant did not exit: $($process.Id)" }
        }
        if (@(Get-CdrWriterOwners $Plan.LockPath).Count -ne 0) {
            throw 'Original process exited but writer ownership is still present; no second process was stopped.'
        }
        Save-CdrDesktopRecoveryReceipt $ReceiptPath @{Phase='restarting'; ThreadId=$Plan.ThreadId; WriterReleased=$true}
        $started=[DateTime]::UtcNow
        $replacement=Start-Process -FilePath $Plan.DesktopPath -WindowStyle Hidden -PassThru
        try {
            $deadline=[DateTime]::UtcNow.AddSeconds(35)
            do {
                $replacement.Refresh()
                if ($replacement.HasExited) { throw 'The replacement desktop exited during startup.' }
                $children=@(Get-CimInstance Win32_Process -Filter "ParentProcessId=$($replacement.Id) AND Name='codex.exe'")
                if ($children.Count -gt 0 -and [DateTime]::UtcNow -gt $started.AddSeconds(3)) { break }
                Start-Sleep -Milliseconds 500
            } while ([DateTime]::UtcNow -lt $deadline)
            if ($children.Count -eq 0) { throw 'Desktop restarted but no replacement Codex backend was observed.' }
            $released=@(Get-CdrWriterOwners $Plan.LockPath).Count -eq 0
            Save-CdrDesktopRecoveryReceipt $ReceiptPath @{
                Phase=$(if ($released) {'completed'} else {'reacquired'})
                ThreadId=$Plan.ThreadId; WriterReleased=$released
                PreviousDesktopIdentity=$Plan.DesktopIdentity
                DesktopIdentity=(Get-CdrRecoveryIdentity $replacement)
                CodexProcessIds=@($children | ForEach-Object { [int]$_.ProcessId })
                FinishedAt=[DateTime]::UtcNow.ToString('o')
            }
        } finally { $replacement.Dispose() }
    } finally {
        if ($null -ne $owned) { foreach ($process in $owned) { $process.Dispose() } }
        else { $desktop.Dispose() }
    }
}
