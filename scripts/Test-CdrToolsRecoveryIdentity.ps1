param([Parameter(Mandatory=$true)][ValidateSet('writer-plan','writer-dispatch','writer-pid-reuse','unchanged','receipt-missing','receipt-empty','receipt-malformed','receipt-null','receipt-array','receipt-same','receipt-same-lf')][string]$Case)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'CdrDesktopRecovery.ps1')
. (Join-Path $PSScriptRoot 'CdrToolsRecovery.ps1')

function Assert-Fixture([bool]$Condition,[string]$Message) {
    if (-not $Condition) { throw $Message }
}
# Every process and OS provider below is fake. Only this test's temp files are used.
$script:events=[Collections.Generic.List[string]]::new()
$script:root=Join-Path ([IO.Path]::GetTempPath()) ('cdr-recover-identity-'+[guid]::NewGuid().ToString('N'))
$script:package=Join-Path $script:root 'package'
$script:fixtureCodexHome=Join-Path $script:root 'home'
$script:desktopPath=Join-Path $script:package 'app\ChatGPT.exe'
$script:runtimePath=Join-Path $script:root 'target\release\cdr-runtime.exe'
$script:writerPath=Join-Path $script:root 'codex.exe'
$script:thread='aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee'
$script:clock=[DateTime]::Parse('2026-09-29T00:00:00Z').ToUniversalTime()

function New-FixtureProcess([int]$Number,[DateTime]$Started,[string]$Path) {
    $p=[pscustomobject]@{Id=$Number;StartTime=$Started;Path=$Path;Handle=1;HasExited=$false;ExitCode=0}
    $p | Add-Member ScriptMethod Dispose {}
    $p | Add-Member ScriptMethod Refresh {}
    $p | Add-Member ScriptMethod CloseMainWindow { $script:events.Add("close:$($this.Id)"); return $false }
    $p | Add-Member ScriptMethod WaitForExit { param($Timeout) return $true }
    $p | Add-Member ScriptMethod Kill { $script:events.Add("kill:$($this.Id)"); $this.HasExited=$true }
    return $p
}
$script:bot=New-FixtureProcess 10 $script:clock $script:runtimePath
$script:desktop=New-FixtureProcess 20 $script:clock.AddSeconds(1) $script:desktopPath
$script:writer=New-FixtureProcess 21 $script:clock.AddSeconds(2) $script:writerPath
$script:replacement=New-FixtureProcess 30 $script:clock.AddSeconds(10) $script:desktopPath
$script:controller=New-FixtureProcess 40 $script:clock.AddSeconds(9) 'fixture-powershell.exe'

function Get-AppxPackage {
    [CmdletBinding()]param([string]$Name)
    Assert-Fixture ($Name -ceq 'OpenAI.Codex') 'Unexpected package lookup.'
    return [pscustomobject]@{InstallLocation=$script:package}
}
function Get-Process {
    [CmdletBinding()]param([int]$Id)
    foreach ($p in @($script:bot,$script:desktop,$script:writer,$script:replacement,$script:controller)) {
        if ($p.Id -eq $Id) { return $p }
    }
    throw "Unexpected fixture process lookup: $Id"
}
function Get-CimInstance {
    [CmdletBinding()]param([string]$ClassName,[string]$Filter,[string[]]$Property)
    Assert-Fixture ($ClassName -ceq 'Win32_Process') 'Unexpected OS provider class.'
    if ($Filter -ceq "Name='ChatGPT.exe'") {
        return [pscustomobject]@{ProcessId=20;ParentProcessId=0;CreationDate=$script:desktop.StartTime;ExecutablePath=$script:desktopPath}
    }
    if ($Filter -ceq "ProcessId=$($script:writer.Id)") {
        return [pscustomobject]@{ProcessId=$script:writer.Id;ParentProcessId=20;CreationDate=$script:writer.StartTime;CommandLine='codex.exe app-server'}
    }
    if ($Filter -ceq "ParentProcessId=30 AND Name='codex.exe'") {
        return [pscustomobject]@{ProcessId=31;ParentProcessId=30;CreationDate=$script:clock.AddSeconds(11);CommandLine='codex.exe app-server'}
    }
    throw "Unexpected fixture OS query: $Filter"
}
function Get-CdrWriterOwners {
    param([string]$LockPath)
    $fileTime=$script:writer.StartTime.ToUniversalTime().ToFileTimeUtc()
    return @([pscustomobject]@{Process=[pscustomobject]@{
        Pid=$script:writer.Id;Started=[pscustomobject]@{
            dwHighDateTime=[uint32]($fileTime -shr 32)
            dwLowDateTime=[uint32]($fileTime -band 0xffffffffL)
        }
    }})
}
function Get-CdrOwnedProcessHandles {
    param($RootProcess)
    Assert-Fixture ($RootProcess.Id -eq 20) 'Unexpected fake tree owner.'
    return ,@($script:desktop,$script:writer)
}
function Start-Process {
    param($FilePath,$WindowStyle,[switch]$PassThru,$ArgumentList)
    Assert-Fixture ($WindowStyle -ceq 'Hidden') 'Helpers must stay hidden.'
    if ($FilePath -ceq $script:desktopPath) {
        $script:events.Add('start:desktop')
        return $script:replacement
    }
    Assert-Fixture ($ArgumentList -contains '-ForceWorker') 'Unexpected external process dispatch.'
    $script:events.Add('start:bot-controller')
    $completed=@{InterruptedIdentity=(Get-CdrRecoveryIdentity $script:bot);ReplacementIdentity='50|639262368100000000'}
    switch ($Case) {
        'receipt-missing' { $completed.Remove('ReplacementIdentity') }
        'receipt-empty' { $completed.ReplacementIdentity='' }
        'receipt-malformed' { $completed.ReplacementIdentity='not-a-process-identity' }
        'receipt-null' { $completed.ReplacementIdentity=$null }
        'receipt-array' { $completed.ReplacementIdentity=@('50|639262368100000000') }
        'receipt-same' { $completed.ReplacementIdentity=$completed.InterruptedIdentity }
        'receipt-same-lf' { $completed.ReplacementIdentity=$completed.InterruptedIdentity+[char]10 }
    }
    [IO.File]::WriteAllText((Join-Path $script:root '.codex_discord_rust.force.completed'),($completed | ConvertTo-Json))
    return $script:controller
}

try {
    [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($script:desktopPath))
    [void][IO.Directory]::CreateDirectory($script:fixtureCodexHome)
    [IO.File]::WriteAllText($script:desktopPath,'fixture only; not executable')
    $epoch=([DateTimeOffset]$script:bot.StartTime).ToUnixTimeSeconds()
    [IO.File]::WriteAllText((Join-Path $script:root '.codex_discord_rust.runtime.lock'),"pid=10"+[Environment]::NewLine+"started_at=$epoch")
    $before=Get-CdrToolsRecoveryPlan $script:root $script:fixtureCodexHome $script:thread
    Assert-Fixture ($before.WriterIdentity -ceq (Get-CdrRecoveryIdentity $script:writer)) 'Initial writer identity was not observed.'
    if ($Case -eq 'writer-pid-reuse') {
        $script:writer=New-FixtureProcess 21 $script:clock.AddSeconds(2).AddTicks(10) $script:writerPath
    } elseif ($Case -in @('writer-plan','writer-dispatch')) {
        $script:writer=New-FixtureProcess 22 $script:clock.AddSeconds(3) $script:writerPath
    }
    $after=Get-CdrToolsRecoveryPlan $script:root $script:fixtureCodexHome $script:thread
    Assert-Fixture ($before.DesktopIdentity -ceq $after.DesktopIdentity -and $before.BotIdentity -ceq $after.BotIdentity) 'Fixture hosts changed unexpectedly.'
    if ($Case -eq 'writer-plan') {
        Assert-Fixture ($before.WriterIdentity -cne $after.WriterIdentity) 'Fixture did not change the writer.'
        Assert-Fixture ($before.RecoveryIdentity -cne $after.RecoveryIdentity) 'Prepared recovery identity ignored a changed writer.'
    } elseif ($Case.StartsWith('receipt-')) {
        $receipt=Join-Path $script:root 'receipt.json'
        Invoke-CdrToolsRestart $before $receipt
        $result=[IO.File]::ReadAllText($receipt) | ConvertFrom-Json
        Write-Output ('completion_phase='+$result.Phase+' bot_identity='+($result.BotIdentity | ConvertTo-Json -Compress))
        Assert-Fixture ($result.Phase -ceq 'failed') 'Unverified bot completion identity was reported as restarted.'
        Assert-Fixture ($result.Error -like '*receipt*') 'Invalid bot receipt lost its diagnostic.'
        Assert-Fixture (-not $result.ToolProbeVerified) 'Invalid bot receipt claimed tool success.'
        Assert-Fixture (($script:events -join ',') -ceq 'close:20,kill:20,kill:21,start:bot-controller,start:desktop') 'Invalid bot receipt skipped desktop restoration or repeated host dispatch.'
    } elseif ($Case -eq 'unchanged') {
        $receipt=Join-Path $script:root 'receipt.json'
        Invoke-CdrToolsRestart $before $receipt
        $result=[IO.File]::ReadAllText($receipt) | ConvertFrom-Json
        Assert-Fixture ($result.Phase -ceq 'restarted') 'Unchanged exact identities did not complete.'
        Assert-Fixture (-not $result.ToolProbeVerified) 'Host restart must not imply tool probe success.'
        Assert-Fixture (($script:events -join ',') -ceq 'close:20,kill:20,kill:21,start:bot-controller,start:desktop') 'Unexpected fake host dispatch sequence.'
    } else {
        Assert-Fixture ($before.WriterIdentity -cne $after.WriterIdentity) 'Fixture did not replace the exact writer.'
        $refused=$false
        try { Invoke-CdrToolsRestart $before (Join-Path $script:root 'receipt.json') }
        catch { $refused=$true; Write-Output ("refusal="+$_.Exception.Message) }
        Write-Output ("fake_process_events="+($script:events -join ','))
        Assert-Fixture ($refused -and $script:events.Count -eq 0) 'Changed writer caused host effects instead of a no-effect refusal.'
    }
    Write-Output "recover_identity_case_passed=$Case"
} finally {
    $resolved=[IO.Path]::GetFullPath($script:root)
    $prefix=[IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')+'\cdr-recover-identity-'
    if (-not $resolved.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase)) { throw 'Fixture cleanup escaped its temp namespace.' }
    if ([IO.Directory]::Exists($resolved)) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
