$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
. (Join-Path $PSScriptRoot 'CdrDesktopRecovery.ps1')
function Assert([bool]$Condition,[string]$Message) { if (-not $Condition) { throw $Message } }
function Must-Reject([scriptblock]$Action) {
    $rejected=$false
    try { & $Action | Out-Null } catch { $rejected=$true }
    Assert $rejected 'Expected refusal was missing.'
}
$temp=Join-Path ([IO.Path]::GetTempPath()) ('cdr-desktop-recovery-test-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory((Join-Path $temp 'thread-writer-locks'))
$thread='aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee'
$owner=$null
try {
    Must-Reject { Get-CdrWriterLockPath $temp '../escape' }
    Assert ((Get-CdrDesktopRecoveryPlan $temp $thread).State -ceq 'unlocked') 'Missing lock must be a no-op.'
    $lock=Get-CdrWriterLockPath $temp $thread
    $child=Join-Path $temp 'owner.ps1'
    [IO.File]::WriteAllText($child,@'
param($LockPath,$ReadyPath)
$stream=[IO.File]::Open($LockPath,'Create','ReadWrite','None')
try { [IO.File]::WriteAllText($ReadyPath,'ready'); Start-Sleep -Seconds 45 }
finally { $stream.Dispose() }
'@)
    $ready=Join-Path $temp 'ready'
    $owner=Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -WindowStyle Hidden -PassThru -ArgumentList @(
        '-NoProfile','-File',('"'+$child+'"'),'-LockPath',('"'+$lock+'"'),'-ReadyPath',('"'+$ready+'"'))
    [void]$owner.Handle
    $deadline=[DateTime]::UtcNow.AddSeconds(5)
    while (-not [IO.File]::Exists($ready) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 50 }
    Assert ([IO.File]::Exists($ready)) 'Fixture owner did not start.'
    $found=@(Get-CdrWriterOwners $lock)
    Assert ($found.Count -eq 1 -and $found[0].Process.Pid -eq $owner.Id) 'Restart Manager did not identify the exact fixture owner.'
    Must-Reject { Get-CdrDesktopRecoveryPlan $temp $thread }
    Assert (-not $owner.HasExited) 'Inspection stopped an unrelated writer.'
    $plan=[pscustomobject]@{ThreadId=$thread;State='desktop_owned';WriterIdentity='1|2';DesktopIdentity='3|4';DesktopPath='app.exe'}
    Assert-CdrSameDesktopOwner $plan $plan
    foreach ($field in @('WriterIdentity','DesktopIdentity','ThreadId','DesktopPath')) {
        $other=$plan | ConvertTo-Json | ConvertFrom-Json
        $other.$field='changed'
        Must-Reject { Assert-CdrSameDesktopOwner $plan $other }
    }
    & {
        $script:killed=[Collections.Generic.List[int]]::new()
        $script:launched=0
        function Fake-Process([int]$Number) {
            $p=[pscustomobject]@{Id=$Number;StartTime=[DateTime]::UtcNow;HasExited=$false;Handle=1}
            $p | Add-Member ScriptMethod Dispose {}
            $p | Add-Member ScriptMethod Refresh {}
            $p | Add-Member ScriptMethod CloseMainWindow { return $false }
            $p | Add-Member ScriptMethod WaitForExit { param($Timeout) return $this.HasExited }
            $p | Add-Member ScriptMethod Kill { $script:killed.Add($this.Id); $this.HasExited=$true }
            return $p
        }
        $desktop=Fake-Process 10; $backend=Fake-Process 11; $replacement=Fake-Process 20
        $fakePlan=[pscustomobject]@{ThreadId=$thread;State='desktop_owned';CodexHome=$temp;LockPath=$lock;
            WriterIdentity=(Get-CdrRecoveryIdentity $backend);DesktopIdentity=(Get-CdrRecoveryIdentity $desktop);DesktopPath='fixture.exe'}
        function Get-CdrDesktopRecoveryPlan { return $fakePlan }
        function Get-Process { return $desktop }
        function Get-CdrOwnedProcessHandles { return ,@($desktop,$backend) }
        function Get-CdrWriterOwners { return @() }
        function Start-Process { $script:launched++; return $replacement }
        function Get-CimInstance { return [pscustomobject]@{ProcessId=21} }
        $receipt=Join-Path $temp 'receipt.json'
        Invoke-CdrDesktopRestart $fakePlan $receipt
        $result=[IO.File]::ReadAllText($receipt) | ConvertFrom-Json
        Assert ($result.Phase -ceq 'completed' -and $result.WriterReleased) 'Successful release was not verified.'
        Assert ($script:killed.Count -eq 2 -and $script:killed.Contains(10) -and $script:killed.Contains(11)) 'Only pinned processes should stop.'
        Assert ($script:launched -eq 1) 'Desktop must be launched once.'
        $script:ownerChecks=0
        function Get-CdrWriterOwners {
            $script:ownerChecks++
            if ($script:ownerChecks -ge 2) { return @([pscustomobject]@{ProcessId=99}) }
            return @()
        }
        Invoke-CdrDesktopRestart $fakePlan $receipt
        $result=[IO.File]::ReadAllText($receipt) | ConvertFrom-Json
        Assert ($result.Phase -ceq 'reacquired' -and -not $result.WriterReleased) 'Reacquired ownership must not report success.'
    }
    Write-Output 'desktop_recovery_tests_passed'
} finally {
    if ($null -ne $owner) {
        if (-not $owner.HasExited) { $owner.Kill(); [void]$owner.WaitForExit(5000) }
        $owner.Dispose()
    }
    $resolved=[IO.Path]::GetFullPath($temp)
    $prefix=[IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')+'\cdr-desktop-recovery-test-'
    if (-not $resolved.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase)) { throw 'Test cleanup path escaped the temp namespace.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
