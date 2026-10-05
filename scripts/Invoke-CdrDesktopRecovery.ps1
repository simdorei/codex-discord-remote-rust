[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][ValidateSet('Inspect','Start','InspectTools','StartTools','Worker')][string]$Mode,
    [Parameter(Mandatory=$true)][string]$RepoRoot,
    [string]$CodexHome,[string]$ThreadId,[string]$ExpectedWriterIdentity,
    [string]$Operation,[string]$RequestHash,[string]$ExpectedRecoveryIdentity
)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$RepoRoot=[IO.Path]::GetFullPath($RepoRoot)
$engine=Join-Path $RepoRoot 'scripts\CdrDesktopRecovery.ps1'
$force=Join-Path $RepoRoot 'scripts\CdrForceRestart.ps1'
. $engine
$toolsEngine=Join-Path $RepoRoot 'scripts\CdrToolsRecovery.ps1'
if ($Mode -in @('InspectTools','StartTools')) { . $toolsEngine }
if ($Mode -eq 'InspectTools') {
    Get-CdrToolsRecoveryPlan $RepoRoot $CodexHome $ThreadId | ConvertTo-Json -Depth 6 -Compress
    exit 0
}
if ($Mode -eq 'Inspect') {
    Get-CdrDesktopRecoveryPlan $CodexHome $ThreadId | ConvertTo-Json -Depth 6 -Compress
    exit 0
}
$base=Join-Path $RepoRoot 'maintenance_backups\desktop-recovery'
if ($Mode -in @('Start','StartTools')) {
    $toolRecovery=$Mode -eq 'StartTools'
    if ($toolRecovery) {
        $plan=Get-CdrToolsRecoveryPlan $RepoRoot $CodexHome $ThreadId
        if ($plan.RecoveryIdentity -cne $ExpectedRecoveryIdentity) { throw 'Prepared tool hosts changed; nothing was stopped.' }
    } else {
        $plan=Get-CdrDesktopRecoveryPlan $CodexHome $ThreadId
        if ($plan.State -cne 'desktop_owned' -or $plan.WriterIdentity -cne $ExpectedWriterIdentity) {
            throw 'Prepared desktop writer no longer matches; nothing was stopped.'
        }
    }
    $Operation=[guid]::NewGuid().ToString('N')
    $bundle=Join-Path $base $Operation
    [void][IO.Directory]::CreateDirectory($bundle)
    $request=Join-Path $bundle 'request.json'
    $receipt=Join-Path $bundle 'receipt.json'
    $payload=@{Version=1;Operation=$Operation;RepoRoot=$RepoRoot;Plan=$plan;ToolRecovery=$toolRecovery
        ExpiresAt=[DateTime]::UtcNow.AddMinutes(2).ToString('o')
        EngineHash=(Get-FileHash -LiteralPath $engine -Algorithm SHA256).Hash
        ForceHash=(Get-FileHash -LiteralPath $force -Algorithm SHA256).Hash
        ControllerHash=(Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash}
    if ($toolRecovery) {
        $payload.ProgramPins=@('scripts/CdrToolsRecovery.ps1','codex-discord-rust-restart.ps1',
            'codex-discord-rust-watchdog.ps1','codex-discord-rust-control.ps1','codex-discord-rust-drain.ps1',
            'scripts/CdrLaunchJournal.ps1') | ForEach-Object {
                @{Path=$_;Hash=(Get-FileHash -LiteralPath (Join-Path $RepoRoot $_) -Algorithm SHA256).Hash}
            }
    }
    [IO.File]::WriteAllText($request,($payload | ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
    $RequestHash=(Get-FileHash -LiteralPath $request -Algorithm SHA256).Hash
    Save-CdrDesktopRecoveryReceipt $receipt @{Phase='dispatched';ThreadId=$ThreadId;Operation=$Operation}
    # The OS provider starts this worker outside both the desktop and bot job trees.
    $shell=Join-Path ([Environment]::GetFolderPath('System')) 'WindowsPowerShell\v1.0\powershell.exe'
    $entry=Join-Path $RepoRoot 'scripts\Invoke-CdrDesktopRecovery.ps1'
    $command='"'+$shell+'" -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "'+
        $entry+'" -Mode Worker -RepoRoot "'+$RepoRoot+'" -Operation '+$Operation+' -RequestHash '+$RequestHash
    $startup=New-CimInstance -ClassName Win32_ProcessStartup -ClientOnly -Property @{ShowWindow=[uint16]0}
    $result=Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
        CommandLine=$command;CurrentDirectory=$RepoRoot;ProcessStartupInformation=$startup
    }
    if ($result.ReturnValue -ne 0 -or $result.ProcessId -le 0) { throw 'Independent desktop recovery worker could not start.' }
    @{Operation=$Operation;ReceiptPath=$receipt;WorkerPid=[int]$result.ProcessId} | ConvertTo-Json -Compress
    exit 0
}
if ($Operation -cnotmatch '^[a-f0-9]{32}$' -or $RequestHash -cnotmatch '^[A-F0-9]{64}$') { throw 'Invalid recovery receipt identity.' }
$bundle=Join-Path $base $Operation
$request=Join-Path $bundle 'request.json'
$receipt=Join-Path $bundle 'receipt.json'
$gate=$null
$claimed=$false
try {
    if ((Get-FileHash -LiteralPath $request -Algorithm SHA256).Hash -cne $RequestHash) { throw 'Recovery request changed.' }
    $payload=[IO.File]::ReadAllText($request) | ConvertFrom-Json
    if ($payload.Version -ne 1 -or $payload.RepoRoot -ine $RepoRoot -or $payload.Operation -cne $Operation -or
        [DateTime]::UtcNow -ge [DateTime]::Parse($payload.ExpiresAt).ToUniversalTime() -or
        (Get-FileHash -LiteralPath $engine -Algorithm SHA256).Hash -cne $payload.EngineHash -or
        (Get-FileHash -LiteralPath $force -Algorithm SHA256).Hash -cne $payload.ForceHash -or
        (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash -cne $payload.ControllerHash) {
        throw 'Recovery request expired or its controller changed.'
    }
    if ($payload.ToolRecovery) {
        $expected=@('scripts/CdrToolsRecovery.ps1','codex-discord-rust-restart.ps1',
            'codex-discord-rust-watchdog.ps1','codex-discord-rust-control.ps1','codex-discord-rust-drain.ps1',
            'scripts/CdrLaunchJournal.ps1')
        if (@($payload.ProgramPins).Count -ne $expected.Count) { throw 'Incomplete recovery program pins.' }
        foreach ($relative in $expected) {
            $pin=@($payload.ProgramPins | Where-Object { $_.Path -ceq $relative })
            if ($pin.Count -ne 1 -or (Get-FileHash -LiteralPath (Join-Path $RepoRoot $relative)).Hash -cne $pin[0].Hash) {
                throw 'Tool recovery controller changed.'
            }
        }
        . $toolsEngine
    }
    $claim=[IO.File]::Open((Join-Path $bundle 'attempted'),'CreateNew','Write','None')
    $claim.Dispose()
    $claimed=$true
    $gate=[IO.File]::Open((Join-Path $base 'desktop.lock'),'OpenOrCreate','ReadWrite','None')
    if (([IO.File]::ReadAllText($receipt) | ConvertFrom-Json).Phase -cne 'dispatched') { throw 'Recovery was already attempted; no automatic retry.' }
    . $force
    Start-Sleep -Seconds 2
    if ($payload.ToolRecovery) { Invoke-CdrToolsRestart $payload.Plan $receipt }
    else { Invoke-CdrDesktopRestart $payload.Plan $receipt }
} catch {
    if ($claimed) {
        Save-CdrDesktopRecoveryReceipt $receipt @{Phase='failed';Operation=$Operation;Error=$_.Exception.Message;FinishedAt=[DateTime]::UtcNow.ToString('o')}
    }
    Write-Error $_ -ErrorAction Continue
    exit 1
} finally { if ($null -ne $gate) { $gate.Dispose() } }
