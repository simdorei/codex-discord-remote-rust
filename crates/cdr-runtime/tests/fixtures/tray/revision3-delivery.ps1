param([string]$Case)
$ErrorActionPreference='Stop'
$ScriptDir=$env:TRAY_CONTRACT_ROOT
$RuntimeMode='rust'
. (Join-Path $ScriptDir 'codex-discord-tray-runtime.ps1')
$script:spawnCount=0
function Get-RustTrayProcess {[pscustomobject]@{ProcessId=42;StartTime=[datetime]'2026-09-01T00:00:00Z'}}
function Test-CdrTrayInteractiveSession {return $true}
function Get-CdrTrayMutexBusy {param($Name);return $false}
function Start-Process {
    [CmdletBinding()]param($FilePath,$ArgumentList,$WorkingDirectory,$WindowStyle,[switch]$PassThru)
    if(($ArgumentList -join ' ') -notmatch 'codex-discord-tray.ps1' -or $ArgumentList -notcontains '-STA'){throw 'NOT_TRAY_ONLY'}
    $script:spawnCount++
    [IO.File]::AppendAllText((Join-Path $ScriptDir 'fixture-spawns.log'),"spawn`n")
    [pscustomobject]@{Id=91}
}
$identity=Get-RustTrayProcessIdentity (Get-RustTrayProcess)
$claim=Join-Path $ScriptDir ('.codex_discord_tray.bootstrap.'+$identity.Replace('|','.')+'.attempted')
$receiptPath=Join-Path $ScriptDir '.codex_discord_rust.restart.completed'
$receipt=[ordered]@{TrayBootstrapRoot=$ScriptDir;TrayBootstrapIdentity=$identity;ReplacementIdentity=$identity}
function SaveReceipt { [IO.File]::WriteAllText($receiptPath,($receipt|ConvertTo-Json -Depth 8)) }
function Deliver {Invoke-CdrTrayAfterWatchdog -Root $ScriptDir -ObservedIdentity $identity}
function AssertState($result,[string]$state,[int]$spawns) {
    if($result.State -cne $state -or $script:spawnCount -ne $spawns){throw "expected=$state/$spawns actual=$($result.State)/$script:spawnCount"}
}
switch($Case) {
    'ordinary' {
        $a=Invoke-CdrTrayAfterWatchdog -Root $ScriptDir -StartedIdentity $identity
        AssertState $a 'spawn_requested' 1
        AssertState (Invoke-CdrTrayAfterWatchdog -Root $ScriptDir -StartedIdentity $identity) 'already_attempted' 1
    }
    'completed' {
        SaveReceipt
        AssertState (Deliver) 'spawn_requested' 1
        AssertState (Deliver) 'already_attempted' 1
    }
    'maintenance_completed' {
        $receiptPath=Join-Path $ScriptDir '.codex_discord_rust.maintenance.v2.completed'
        $receipt=[ordered]@{Version=2;Phase='verified';RepoRoot=$ScriptDir;BinaryPath=(Join-Path $ScriptDir 'target\release\cdr-runtime.exe');TrayBootstrapIdentity=$identity;CompletionPolicy='runtime-proof-v1';RuntimeEvidence=@{ChildIdentity=$identity}}
        SaveReceipt
        AssertState (Deliver) 'spawn_requested' 1
        AssertState (Deliver) 'already_attempted' 1
    }
    'no_intent' {
        $receipt.Remove('TrayBootstrapIdentity');SaveReceipt
        AssertState (Deliver) 'no_request' 0
        if([IO.File]::Exists($claim)){throw 'legacy receipt minted attempt'}
    }
    'wrong_root' {$receipt.TrayBootstrapRoot=Join-Path $ScriptDir 'other';SaveReceipt;AssertState (Deliver) 'no_request' 0}
    'wrong_child' {$receipt.ReplacementIdentity='43|1';SaveReceipt;AssertState (Deliver) 'no_request' 0}
    'empty_claim' {SaveReceipt;[IO.File]::WriteAllText($claim,'');AssertState (Deliver) 'already_attempted' 0;if((Get-Item $claim).Length){throw 'empty claim changed'}}
    'unknown_spawn' {
        SaveReceipt
        function Start-Process {[CmdletBinding()]param($FilePath,$ArgumentList,$WorkingDirectory,$WindowStyle,[switch]$PassThru);$script:spawnCount++;throw 'fixture unknown spawn'}
        AssertState (Deliver) 'unknown' 1
        AssertState (Deliver) 'already_attempted' 1
    }
    'owner_closed' {
        SaveReceipt
        function Get-CdrTrayMutexBusy {param($Name);return $true}
        AssertState (Deliver) 'owner_present' 0
        function Get-CdrTrayMutexBusy {param($Name);return $false}
        AssertState (Deliver) 'already_attempted' 0
    }
    'noninteractive' {
        SaveReceipt;function Test-CdrTrayInteractiveSession {return $false}
        AssertState (Deliver) 'noninteractive' 0
        function Test-CdrTrayInteractiveSession {return $true}
        AssertState (Deliver) 'already_attempted' 0
    }
    'markers' {
        SaveReceipt
        foreach($name in @('.codex_discord_bot.disabled','.codex_discord_rust.stop','.codex_discord_rust.restart','.codex_discord_rust.restart.launch','.codex_discord_rust.drain.prepare','.codex_discord_rust.drain.ack','.codex_discord_rust.maintenance.v2','.codex_discord_rust.restart.claimed.123.456')) {
            $marker=Join-Path $ScriptDir $name;[IO.File]::WriteAllText($marker,'fixture')
            AssertState (Deliver) 'control_pending' 0
            if([IO.File]::Exists($claim)){throw 'pending control consumed attempt'}
            [IO.File]::Delete($marker)
        }
    }
    'marker_race' {
        SaveReceipt
        function Get-CdrTrayMutexBusy {param($Name);[IO.File]::WriteAllText((Join-Path $ScriptDir '.codex_discord_rust.stop'),'new owner');return $false}
        AssertState (Deliver) 'control_pending' 0
        [IO.File]::Delete((Join-Path $ScriptDir '.codex_discord_rust.stop'))
        AssertState (Deliver) 'already_attempted' 0
    }
    'identity_race' {
        SaveReceipt
        function Get-CdrTrayMutexBusy {param($Name);$script:changed=$true;return $false}
        function Get-RustTrayProcess {$id=if($script:changed){43}else{42};[pscustomobject]@{ProcessId=$id;StartTime=[datetime]'2026-09-01T00:00:00Z'}}
        AssertState (Deliver) 'runtime_mismatch' 0
        $script:changed=$false
        AssertState (Deliver) 'already_attempted' 0
    }
    'worker' {
        $r=Invoke-CdrTrayAfterWatchdog -Root $ScriptDir -StartedIdentity $identity
        if($r.State -notin @('spawn_requested','already_attempted')){throw "unexpected concurrent result:$($r.State)"}
    }
    default {throw "unknown case:$Case"}
}
Write-Output "PASS delivery=$Case spawns=$script:spawnCount"
