param([string]$Case)
$ErrorActionPreference='Stop'
$ScriptDir=$env:TRAY_CONTRACT_ROOT
$RuntimeMode='rust'
. (Join-Path $ScriptDir 'codex-discord-tray-runtime.ps1')
$lockPath=Join-Path $ScriptDir '.codex_discord_rust.runtime.lock'
[IO.File]::WriteAllText($lockPath,"pid=42`n")
function Start-Process { throw 'UNEXPECTED_PROCESS_START' }
function Get-CimInstance { throw 'UNEXPECTED_PROCESS_SCAN' }
function GoodProcess {
    [pscustomobject]@{Id=42;Path=(Join-Path $ScriptDir 'target\release\cdr-runtime.exe');StartTime=[datetime]'2026-09-01T00:00:00Z'}
}
function Get-Process { [CmdletBinding()]param($Id); GoodProcess }
$expected='unknown'
switch ($Case) {
    'missing_lock' { [IO.File]::Delete($lockPath);$expected='stopped' }
    'wrong_path' { function Get-Process {[CmdletBinding()]param($Id);$p=GoodProcess;$p.Path=Join-Path $ScriptDir 'other.exe';$p};$expected='stopped' }
    'pid_absent' {
        [IO.File]::WriteAllText($lockPath,"pid=2147483647`n")
        Remove-Item Function:Get-Process
        $expected='stopped'
    }
    'valid' {$expected='running'}
    'empty' {[IO.File]::WriteAllText($lockPath,'')}
    'incomplete' {[IO.File]::WriteAllText($lockPath,'pid=')}
    'zero' {[IO.File]::WriteAllText($lockPath,'pid=0')}
    'overflow' {[IO.File]::WriteAllText($lockPath,'pid=999999999999999999')}
    'duplicate' {[IO.File]::WriteAllText($lockPath,"pid=42`npid=42`n")}
    'mixed_duplicate' {[IO.File]::WriteAllText($lockPath,"pid=42`npid=broken`n")}
    'directory_lock' {[IO.File]::Delete($lockPath);$null=[IO.Directory]::CreateDirectory($lockPath)}
    'read_denied' {
        function Get-Content {[CmdletBinding()]param($LiteralPath,[switch]$Raw,$Encoding);throw [UnauthorizedAccessException]::new('fixture lock read denied')}
    }
    'item_denied' {
        function Get-Item {[CmdletBinding()]param($LiteralPath);throw [UnauthorizedAccessException]::new('fixture item denied')}
    }
    'process_denied' {function Get-Process {[CmdletBinding()]param($Id);throw [UnauthorizedAccessException]::new('fixture process denied')}}
    'process_nonterminating' {function Get-Process {[CmdletBinding()]param($Id);Write-Error 'fixture process query error'}}
    'process_empty' {function Get-Process {[CmdletBinding()]param($Id);return $null}}
    'path_empty' {function Get-Process {[CmdletBinding()]param($Id);$p=GoodProcess;$p.Path='';$p}}
    'time_empty' {function Get-Process {[CmdletBinding()]param($Id);$p=GoodProcess;$p.StartTime=$null;$p}}
    'wrong_id' {function Get-Process {[CmdletBinding()]param($Id);$p=GoodProcess;$p.Id=43;$p}}
    'getter_error' {
        function Get-Process {[CmdletBinding()]param($Id);$p=GoodProcess;$p.PSObject.Properties.Remove('Path');$p|Add-Member ScriptProperty Path {throw 'fixture path getter error'};$p}
    }
    'once' {
        [IO.File]::WriteAllText($lockPath,'pid=')
        & (Join-Path $ScriptDir 'codex-discord-tray.ps1') -Once
        exit $LASTEXITCODE
    }
    'recovery' {
        function Get-Process {[CmdletBinding()]param($Id);return $null}
        $a=Get-BotStatus
        if($a.State -cne 'unknown' -or $null -ne $a.Pid){throw 'uncertain observation reused a PID'}
        function Get-Process {[CmdletBinding()]param($Id);GoodProcess}
        $expected='running'
    }
    default {throw "unknown fixture case:$Case"}
}
$status=Get-BotStatus
if($status.State -cne $expected){throw "case=$Case expected=$expected actual=$($status.State)"}
if($expected -ne 'running' -and ($null -ne $status.Pid -or $status.Running)){throw 'non-running observation retained PID'}
if($expected -eq 'running' -and $status.Pid -ne 42){throw 'wrong running PID'}
Write-Output "PASS observation=$Case state=$expected"
